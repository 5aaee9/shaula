import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { randomBytes, randomUUID, createHash } from "node:crypto";
import { mkdtemp, writeFile, readFile, open, mkdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { issuer, scopes } from "./oidc.mjs";
import { faultProxy } from "./faults.mjs";
import { command, freePort, json, until, serverImage, runnerImage } from "./support.mjs";

export class Fixture {
  prefix = `shaula-lifecycle-${randomUUID().slice(0, 8)}`;
  containers = new Set();
  fleets = new Set();
  docker = process.env.DOCKER_BIN || "docker";
  socket = process.env.DOCKER_HOST || "unix:///var/run/docker.sock";
  binary = resolve(process.env.SHAULA_BIN || "target/debug/shaula");
  terraform = process.env.TERRAFORM_BIN || "terraform";
  user = "lifecycle-test";
  platform = "docker";

  async cli(...args) { return command(this.docker, ["--host", this.socket, ...args]); }
  api(path, method, body, expected, headers) {
    return json(`${this.url}/api/v1${path}`, this.oidc.token, method, body, expected, headers);
  }
  forgejo(path, method, body, expected) {
    return json(`${this.direct}/api/v1${path}`, this.token, method, body, expected);
  }
  async start() {
    assert(this.socket.startsWith("unix:///"), "acceptance requires an explicit local Docker socket");
    this.engine = JSON.parse(await this.cli("version", "--format", "{{json .Server}}"));
    assert(this.engine.Components?.some(c => c.Name === "Engine" && c.Details?.Os === "linux"), "real Linux Docker Engine required (not Podman)");
    this.directory = await mkdtemp(join(tmpdir(), `${this.prefix}-`));
    const bridge = JSON.parse(await this.cli("network", "inspect", "bridge"))[0];
    const gateway = bridge.IPAM.Config[0].Gateway;
    assert.match(gateway, /^\d+\.\d+\.\d+\.\d+$/);
    // Run the harness in the Docker host network namespace. A rootless daemon
    // needs nsenter; the probe fails explicitly rather than changing templates.
    const port = await freePort();
    this.direct = `http://${gateway}:${port}`;
    await this.cli("pull", serverImage);
    await this.cli("pull", runnerImage);
    this.server = await this.cli("create", "--name", this.prefix, "--network", "host",
      "-e", "FORGEJO__actions__ENABLED=true", "-e", "FORGEJO__security__INSTALL_LOCK=true",
      "-e", "FORGEJO__service__DISABLE_REGISTRATION=true", "-e", "FORGEJO__server__DISABLE_SSH=true",
      "-e", `FORGEJO__server__HTTP_ADDR=${gateway}`, "-e", `FORGEJO__server__HTTP_PORT=${port}`,
      "-e", `FORGEJO__server__ROOT_URL=${this.direct}/`, serverImage);
    this.containers.add(this.server);
    await this.cli("start", this.server);
    await until("Forgejo ready in Docker host namespace", async () => {
      try { return (await fetch(`${this.direct}/api/v1/version`, { signal: AbortSignal.timeout(2000) })).ok; } catch { return false; }
    });
    await this.cli("exec", "--user", "git", this.server, "forgejo", "admin", "user", "create", "--username", this.user,
      "--random-password", "--email", "lifecycle@example.invalid", "--admin", "--must-change-password=false");
    this.token = await this.cli("exec", "--user", "git", this.server, "forgejo", "admin", "user", "generate-access-token",
      "--username", this.user, "--token-name", "disposable", "--scopes", "all", "--raw");
    assert(/^[a-f0-9]{40}$/.test(this.token), "generated administrator token format");
    const relayPort = await freePort();
    this.target = `http://${gateway}:${relayPort}`;
    this.registrationProxy = await faultProxy({ target: this.direct, listen: { port: relayPort, host: gateway } });
    this.proxySocket = join(this.directory, "docker.sock");
    this.dockerProxy = await faultProxy({ socketPath: this.socket.slice(7), listen: this.proxySocket });
    this.oidc = await issuer(this.directory);
    this.port = await freePort();
    this.url = `http://127.0.0.1:${this.port}`;
    this.config = {
      version: 1, storage: { data_dir: join(this.directory, "data") },
      http: { listen: `127.0.0.1:${this.port}`, bindings_server_key: randomBytes(32).toString("hex"),
        authorization: [{ issuer: this.oidc.url, subject: "fixture", scopes: scopes.split(" ") }] },
      execution: { engines: { terraform: { executable: this.terraform } }, operation_timeout_secs: 90 },
      runner: { max_lifetime_secs: 300 },
    };
    await this.startDaemon();
    await this.publish();
  }

  async startDaemon() {
    const path = join(this.directory, "config.json");
    await writeFile(path, JSON.stringify(this.config), { mode: 0o600 });
    const log = await open(join(this.directory, "daemon.log"), "a", 0o600);
    this.child = spawn(this.binary, ["serve", "--config", path], { stdio: ["ignore", log.fd, log.fd], env: {
      ...process.env, SHAULA_OIDC_PROVIDER: this.oidc.url, SHAULA_OIDC_CLIENT_ID: "fixture", SHAULA_OIDC_CLIENT_SECRET: "fixture-only",
      SHAULA_OIDC_PUBLIC_URL: this.publicUrl || "https://shaula.example.invalid", SHAULA_OIDC_API_AUDIENCE: "shaula-api", SHAULA_OIDC_CA_CERT: this.oidc.certificate,
    } });
    this.child.on("error", () => { this.spawnFailed = true; });
    await log.close();
    await until("shaula serve ready", async () => {
      assert(!this.spawnFailed && this.child.exitCode === null && this.child.signalCode === null, "shaula exited; inspect protected fixture log");
      try { return (await fetch(`${this.url}/readyz`, { headers: { authorization: `Bearer ${this.oidc.token}` }, signal: AbortSignal.timeout(2000) })).ok; } catch { return false; }
    });
  }
  async stopDaemon() {
    if (this.child && this.child.exitCode === null && this.child.signalCode === null) {
      const stopped = new Promise(resolve => this.child.once("exit", resolve));
      this.child.kill("SIGKILL");
      await stopped;
    }
  }
  async restart(lifetime = this.config.runner.max_lifetime_secs) {
    await this.stopDaemon();
    this.config.runner.max_lifetime_secs = lifetime;
    await this.startDaemon();
  }
  async publish() {
    const archive = join(this.directory, `${this.platform}.tar.gz`);
    await command("tar", ["-czf", archive, "-C", resolve(`templates/${this.platform}`), "profile.yaml", "main.tf", ".terraform.lock.hcl", "runtime-policy.md", "schemas"]);
    const bytes = await readFile(archive);
    this.digest = `sha256:${createHash("sha256").update(bytes).digest("hex")}`;
    const response = await fetch(`${this.url}/api/v1/template-artifacts/${this.digest}`, { method: "PUT", headers: { authorization: `Bearer ${this.oidc.token}` }, body: bytes, signal: AbortSignal.timeout(30_000) });
    assert.equal(response.status, 201, "artifact publication");
    await this.api("/template-profiles/forgejo", "PUT", {
      artifact_digest: this.digest, engine_ref: "terraform", bindings: this.bindings(), fleet_input_policy: {},
    }, 202, { "if-none-match": "*" });
    await this.api("/github-auth-profiles/forgejo", "PUT", { kind: "forgejo_token", instance_url: this.target, scope: { kind: "instance" }, token: this.token }, 202, { "if-none-match": "*" });
    for (const kind of ["template-profiles", "github-auth-profiles"]) {
      await until(`${kind} Active`, async () => {
        const profile = await this.api(`/${kind}/forgejo`);
        assert(!["Rejected", "Unsupported"].includes(profile.status), `${kind}: rejected`);
        return profile.activeRevision === 1;
      });
    }
  }
  bindings() { return { docker_host: `unix://${this.proxySocket}`, runner_backend: "forgejo" }; }
  async createFleet(key, min = 0, max = 1, labels = [`${key}:host`]) {
    this.fleets.add(key);
    await this.api(`/fleets/${key}`, "PUT", { kind: "forgejo", forgejo: {
      instance_url: this.target, scope: { kind: "instance" }, auth_profile_ref: "forgejo", runner_name_prefix: `${key}-`, labels,
    }, capacity: { min_runners: min, max_runners: max }, template_profile_ref: "forgejo", template_inputs: {} }, 202, { "if-none-match": "*" });
  }
  async capZero(key) {
    const response = await fetch(`${this.url}/api/v1/fleets/${key}`, { headers: { authorization: `Bearer ${this.oidc.token}` } });
    assert.equal(response.status, 200);
    const body = await response.json();
    body.spec.capacity = { min_runners: 0, max_runners: 0 };
    await this.api(`/fleets/${key}`, "PUT", body.spec, 202, { "if-match": response.headers.get("shaula-resource-version") || response.headers.get("etag") });
  }
  async queue(key, seconds = 8, failure = false, labels = [key]) {
    await this.forgejo("/user/repos", "POST", { name: key, private: true, auto_init: true, default_branch: "main" }, 201);
    const yaml = `name: lifecycle\non: [push]\njobs:\n  build:\n    runs-on: [${labels.join(", ")}]\n    steps:\n      - run: |\n          sleep ${seconds}\n          exit ${failure ? 1 : 0}\n`;
    await this.forgejo(`/repos/${this.user}/${key}/contents/.forgejo/workflows/check.yaml`, "POST", { content: Buffer.from(yaml).toString("base64"), message: "disposable lifecycle acceptance", branch: "main" }, 201);
    await until("queued demand", async () => (await this.forgejo(`/admin/actions/runners/jobs?labels=${labels.join(",")}`) ?? []).some(j => j.status === "waiting"));
  }
  async containerVolumes(id) {
    const [container] = JSON.parse(await this.cli("inspect", id));
    return (container.Mounts || []).filter(m => m.Type === "volume").map(m => m.Name);
  }
  async observe(key, status) {
    return until(`${key}: runner ${status}`, async () => {
      const rows = await this.api(`/generations?fleet_key=${key}`);
      assert(rows.items.every(g => !["Quarantined", "CleanupRequired"].includes(g.state)), "unexpected quarantined/create failure; inspect protected daemon log");
      const runners = await this.forgejo("/admin/actions/runners");
      const runner = runners.find(r => r.name.startsWith(`${key}-`) && r.status === status);
      if (!runner) return false;
      assert.equal(runner.ephemeral, true);
      const ids = await this.cli("ps", "-a", "--no-trunc", "-q", "--filter", `label=shaula.fleet=${key}`);
      if (!ids) return false;
      assert.equal(ids.split("\n").length, 1, "one demand must produce exactly one container");
      this.containers.add(ids);
      const volumes = await this.containerVolumes(ids);
      return { runner: runner.id, id: ids, volumes };
    });
  }
  async reclaimed(key, observed) {
    await until(`${key}: full reclaim`, async () => {
      const status = await this.api(`/fleets/${key}/status`);
      return status.capacity.occupancy === 0;
    });
    const generations = await this.api(`/generations?fleet_key=${key}`);
    assert.equal(generations.items.length, 1, "no duplicate generation after restart/retry");
    assert.equal(generations.items[0].state, "Destroyed");
    assert(!((await this.forgejo("/admin/actions/runners")).some(r => r.id === observed.runner)), "registration remains");
    assert(!(await this.cli("ps", "-a", "-q", "--no-trunc")).split("\n").includes(observed.id), "container remains");
    const volumes = (await this.cli("volume", "ls", "-q")).split("\n");
    assert(observed.volumes.every(v => !volumes.includes(v)), "credential volume remains");
    this.containers.delete(observed.id);
  }
  async close() {
    await this.stopDaemon();
    // Only resources bearing this fixture's exact random Fleet keys. Teardown
    // is never evidence of successful Shaula cleanup; assertions precede it.
    for (const key of this.fleets) {
      const ids = await this.cli("ps", "-a", "--no-trunc", "-q", "--filter", `label=shaula.fleet=${key}`);
      for (const id of ids.split("\n").filter(Boolean)) this.containers.add(id);
    }
    const remaining = new Set((await this.cli("ps", "-a", "--no-trunc", "-q")).split("\n"));
    for (const id of [...this.containers].reverse()) {
      if (remaining.has(id)) await this.cli("rm", "-f", "-v", id);
    }
    await this.dockerProxy?.close();
    await this.registrationProxy?.close();
    await this.oidc?.close();
    // Keep private evidence on failure and success; no automatic deletion of state.
    if (this.directory) await mkdir(this.directory, { recursive: true, mode: 0o700 });
  }
}
