// The same lifecycle on the bundled Kubernetes template in a disposable Kind
// cluster. Only a caller-provisioned, explicitly named kind-shaula-* context
// is accepted; teardown deletes only the exact namespace this run created.
import assert from "node:assert/strict";
import { createServer, connect } from "node:net";
import { resolve } from "node:path";
import { GitlabFixture } from "./fixture.mjs";
import { command, freePort, until } from "../forgejo-lifecycle/support.mjs";

export class KubernetesGitlabFixture extends GitlabFixture {
  platform = "kubernetes";
  sockets = new Set();

  async kube(...args) {
    return command(process.env.KUBECTL_BIN || "kubectl", ["--kubeconfig", this.kubeconfig, "--context", this.context, "--namespace", this.prefix, ...args]);
  }
  logs(id) { return this.kube("logs", id); }

  async start() {
    assert(process.env.KUBECONFIG && process.env.SHAULA_ACCEPTANCE_KUBE_CONTEXT, "explicit disposable kubeconfig and context required");
    this.kubeconfig = resolve(process.env.KUBECONFIG);
    this.context = process.env.SHAULA_ACCEPTANCE_KUBE_CONTEXT;
    assert(this.context.startsWith("kind-shaula-"), "acceptance only supports an isolated shaula Kind context");
    await super.start();
  }

  // Kind Pods reach host-local addresses (the Forgejo Kubernetes path relies on
  // the same route) but not necessarily another bridge's DNAT target. GitLab
  // is therefore published on loopback only, and an in-process TCP relay on
  // the Docker bridge gateway serves the one external_url to the daemon and
  // to Generation Pods alike.
  async endpoint(gateway) {
    const port = await freePort(gateway);
    const local = await freePort();
    this.relay = createServer(client => {
      const upstream = connect(local, "127.0.0.1");
      for (const socket of [client, upstream]) {
        this.sockets.add(socket);
        socket.on("close", () => this.sockets.delete(socket));
        socket.on("error", () => { client.destroy(); upstream.destroy(); });
      }
      client.pipe(upstream).pipe(client);
    });
    await new Promise((done, fail) => { this.relay.once("error", fail); this.relay.listen(port, gateway, done); });
    return { port, publish: `127.0.0.1:${local}:${port}` };
  }

  async publish() {
    this.runtimeTuple.kubernetesVersion = JSON.parse(await this.kube("version", "-o", "json")).serverVersion.gitVersion;
    await this.kube("create", "namespace", this.prefix);
    this.namespaceCreated = true;
    await super.publish();
  }
  bindings() { return { runner_backend: "gitlab", namespace: this.prefix, kubeconfig: this.kubeconfig }; }

  async runningGeneration(key, rows) {
    const pods = JSON.parse(await this.kube("get", "pods", "-l", `shaula.io/fleet=${key}`, "-o", "json")).items;
    if (!pods.length) return false;
    assert.equal(pods.length, 1, "one matching job must produce exactly one Pod");
    const [pod] = pods;
    assert(!["Failed", "Succeeded"].includes(pod.status.phase), `runner Pod ${pod.status.phase} before claiming`);
    if (pod.status.phase !== "Running") return false;
    const secret = JSON.parse(await this.kube("get", "secret", pod.metadata.name, "-o", "json"));
    return { id: pod.metadata.name, uid: pod.metadata.uid, secretUid: secret.metadata.uid, generation: rows.items[0].id };
  }

  async assertCreated(observed) {
    const pod = JSON.parse(await this.kube("get", "pod", observed.id, "-o", "json"));
    assert.equal(pod.metadata.uid, observed.uid);
    assert.equal(pod.spec.restartPolicy, "Never");
    assert.equal(pod.spec.automountServiceAccountToken, false);
    assert.equal(pod.spec.containers.length, 1);
    assert(!pod.spec.initContainers?.length, "no init container");
    const [runner] = pod.spec.containers;
    assert.equal(runner.image, this.runnerImage);
    assert.deepEqual(runner.command, ["/usr/bin/dumb-init", "--", "/usr/bin/gitlab-runner"]);
    assert.equal(runner.args[0], "run-single");
    assert.equal(runner.securityContext.runAsUser, 999);
    assert.equal(runner.securityContext.runAsNonRoot, true);
    assert.equal(runner.securityContext.allowPrivilegeEscalation, false);
    assert.deepEqual(runner.securityContext.capabilities.drop, ["ALL"]);
    assert.equal(pod.spec.volumes.length, 2);
    assert(pod.spec.volumes.every(v => (v.secret?.secretName === observed.id) !== (v.emptyDir !== undefined)),
      "only the bootstrap Secret item and the emptyDir home; no service-account token, host path or socket");
    assert.equal(pod.status.containerStatuses[0].restartCount, 0);
    const secret = JSON.parse(await this.kube("get", "secret", observed.id, "-o", "json"));
    assert.equal(secret.immutable, true, "bootstrap Secret frozen after identity-checked release");
    assert.deepEqual(Object.keys(secret.data), ["config.toml"]);
    // run-single persisted its system ID beside the read-only config.toml.
    await until("runner system ID persisted in the writable home", async () => {
      try { return /^[rs]_[A-Za-z0-9]{12}$/.test(await this.kube("exec", observed.id, "--", "cat", "/home/gitlab-runner/.runner_system_id")); }
      catch { return false; }
    }, 30_000);
    return "one Pod + immutable config.toml Secret; official image as UID 999, restartPolicy Never, no SA token, writable emptyDir home";
  }

  async deliveredConfig(observed) {
    const secret = JSON.parse(await this.kube("get", "secret", observed.id, "-o", "json"));
    return Buffer.from(secret.data["config.toml"], "base64").toString();
  }

  async runtimeSurfaces(observed) {
    const secret = JSON.parse(await this.kube("get", "secret", observed.id, "-o", "json"));
    assert.equal(secret.immutable, true, "bootstrap Secret frozen after identity-checked release");
    return [["Pod argv/environment/metadata", await this.kube("get", "pod", observed.id, "-o", "json"), true],
      ["Secret metadata", JSON.stringify(secret.metadata), true],
      ["runner log", await this.logs(observed.id), false]];
  }

  async assertDestroyed(observed) {
    for (const kind of ["pod", "secret"]) {
      assert.equal(await this.kube("get", kind, observed.id, "--ignore-not-found", "-o", "name"), "", `${kind} remains`);
    }
    return "registration deleted after pause → recheck; Pod and credential Secret destroyed";
  }

  // Operator-approved Kubernetes boundary (as for Forgejo): provider refresh
  // can retain the released config.toml in the original bootstrap Secret's
  // state data. Nowhere else, and never the group access token.
  protectedState(body, observed, secrets) {
    const parsed = JSON.parse(body);
    const walk = value => {
      if (!value || typeof value !== "object") return;
      if (value.type === "kubernetes_secret_v1" && value.name === "bootstrap") {
        for (const instance of value.instances || []) {
          const data = instance.attributes?.data;
          if (!data?.["config.toml"]) continue;
          const metadata = instance.attributes.metadata?.[0];
          assert(metadata?.name === observed.id && metadata.namespace === this.prefix && metadata.uid === observed.secretUid,
            "retained bootstrap Secret must have the original identity");
          assert(!data["config.toml"].includes(secrets[0]), "credential leak in retained Secret data");
          data["config.toml"] = "[single-runner config.toml in protected state]";
        }
      }
      for (const child of Object.values(value)) walk(child);
    };
    walk(parsed);
    return JSON.stringify(parsed);
  }

  async close() {
    try { await super.close(); }
    finally {
      try { if (this.namespaceCreated) await this.kube("delete", "namespace", this.prefix, "--wait=true", "--timeout=120s"); }
      finally {
        for (const socket of this.sockets) socket.destroy();
        if (this.relay) await new Promise(done => this.relay.close(done));
      }
    }
  }
}
