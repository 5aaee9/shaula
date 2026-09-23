import assert from "node:assert/strict";
import { resolve } from "node:path";
import { Fixture } from "./fixture.mjs";
import { command, until } from "./support.mjs";

// Only a caller-provisioned, explicitly named disposable context is accepted.
export class KubernetesFixture extends Fixture {
  platform = "kubernetes";
  async kube(...args) {
    return command(process.env.KUBECTL_BIN || "kubectl", ["--kubeconfig", this.kubeconfig, "--context", this.context, "--namespace", this.prefix, ...args]);
  }
  async publish() {
    assert(process.env.KUBECONFIG && process.env.SHAULA_ACCEPTANCE_KUBE_CONTEXT, "explicit disposable kubeconfig and context required");
    this.kubeconfig = resolve(process.env.KUBECONFIG);
    this.context = process.env.SHAULA_ACCEPTANCE_KUBE_CONTEXT;
    assert(this.context.startsWith("kind-shaula-"), "acceptance only supports an isolated shaula Kind context");
    await this.kube("create", "namespace", this.prefix);
    this.namespaceCreated = true;
    await super.publish();
  }
  bindings() { return { runner_backend: "forgejo", namespace: this.prefix, kubeconfig: this.kubeconfig }; }
  async observe(key, status) {
    return until(`${key}: Kubernetes runner ${status}`, async () => {
      const rows = await this.api(`/generations?fleet_key=${key}`);
      assert(rows.items.every(g => !["Quarantined", "CleanupRequired"].includes(g.state)), "unexpected Kubernetes Create failure; inspect protected daemon log");
      const runner = (await this.forgejo("/admin/actions/runners")).find(r => r.name.startsWith(`${key}-`) && r.status === status);
      if (!runner) return false;
      assert.equal(runner.ephemeral, true);
      const pods = JSON.parse(await this.kube("get", "pods", "-l", `shaula.io/fleet=${key}`, "-o", "json")).items;
      assert.equal(pods.length, 1, "one demand must produce one Pod");
      const pod = pods[0];
      assert.equal(pod.spec.restartPolicy, "Never");
      assert.equal(pod.spec.automountServiceAccountToken, false);
      assert(pod.spec.volumes.every(v => !v.projected?.sources?.some(s => s.serviceAccountToken)), "no service-account credential mount");
      assert.equal(pod.spec.containers[0].securityContext.runAsNonRoot, true);
      assert.equal(pod.spec.containers[0].securityContext.allowPrivilegeEscalation, false);
      const secret = JSON.parse(await this.kube("get", "secret", pod.metadata.name, "-o", "json"));
      assert.equal(secret.immutable, true);
      assert(secret.data.token, "host released token after Create");
      return { id: pod.metadata.name, uid: pod.metadata.uid, secretUid: secret.metadata.uid, runner: runner.id };
    }, 180_000);
  }
  async reclaimed(key, observed) {
    await until(`${key}: Pod, Secret and registration reclaimed`, async () => (await this.api(`/fleets/${key}/status`)).capacity.occupancy === 0);
    const generations = await this.api(`/generations?fleet_key=${key}`);
    assert.equal(generations.items.length, 1);
    assert.equal(generations.items[0].state, "Destroyed");
    assert(!(await this.forgejo("/admin/actions/runners")).some(r => r.id === observed.runner));
    const resources = JSON.parse(await this.kube("get", "pods,secrets", "-l", `shaula.io/fleet=${key}`, "-o", "json"));
    assert.equal(resources.items.length, 0, "Pod or credential Secret remains");
  }
  async close() {
    try { await super.close(); }
    finally { if (this.namespaceCreated) await this.kube("delete", "namespace", this.prefix, "--wait=true", "--timeout=60s"); }
  }
}
