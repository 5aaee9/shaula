// Disposable GitLab CE + actual `shaula serve` + real Terraform/Docker.
// Never targets an existing GitLab; never prunes or searches by name: teardown
// removes only the exact container IDs this fixture created or observed.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { randomBytes, randomUUID, createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { mkdtemp, writeFile, readFile, open, rm } from "node:fs/promises";
import { tmpdir, release } from "node:os";
import { join, resolve } from "node:path";
import { issuer, scopes } from "../forgejo-lifecycle/oidc.mjs";
import { fenceFixture } from "../forgejo-lifecycle/worker-fence.mjs";
import { command, containerLogs, freePort, json, until } from "../forgejo-lifecycle/support.mjs";


export const serverImage = "gitlab/gitlab-ce:19.4.2-ce.0";

/** The exact pinned GitLab image from the bundled platform profile. */
export async function runnerImage(platform = "docker") {
  const profile = await readFile(resolve(`templates/${platform}/profile.yaml`), "utf8");
  const image = profile.split("\n").map(line => line.trim().replace(/^- /, ""))
    .find(line => line.startsWith("gitlab/gitlab-runner:"));
  assert.match(image ?? "", /^gitlab\/gitlab-runner:v[0-9.]+@sha256:[a-f0-9]{64}$/);
  return image;
}

export class GitlabFixture {
  prefix = `shaula-gitlab-${randomUUID().slice(0, 8)}`;
  containers = new Set();
  fleets = new Set();
  docker = process.env.DOCKER_BIN || "docker";
  socket = process.env.DOCKER_HOST || "unix:///var/run/docker.sock";
  binary = resolve(process.env.SHAULA_BIN || "target/debug/shaula");
  terraform = process.env.TERRAFORM_BIN || "terraform";
  platform = "docker";
  /** URL the daemon and Generation runners use; a fault-proxy subclass overrides it. */
  get endpoint() { return this.instance; }

  async cli(...args) { return command(this.docker, ["--host", this.socket, ...args]); }
  /** Container output on both streams; `docker logs` replays stderr on stderr. */
  logs(id) { return containerLogs(this.docker, this.socket, id); }
  api(path, method, body, expected, headers) {
    return json(`${this.url}/api/v1${path}`, this.oidc.token, method, body, expected, headers);
  }
  async gitlab(path, { method = "GET", body, token = this.adminToken, expected = 200 } = {}) {
    const response = await fetch(`${this.instance}/api/v4${path}`, {
      method, headers: { "PRIVATE-TOKEN": token, "content-type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body), redirect: "error", signal: AbortSignal.timeout(15_000),
    });
    // Status and path only; GitLab bodies can echo credentials.
    assert.equal(response.status, expected, `${method} ${path.split("?")[0]}: HTTP status`);
    const text = await response.text();
    return path.endsWith("/trace") ? text : text ? JSON.parse(text) : null;
  }

  async start() {
    assert(this.socket.startsWith("unix:///"), "acceptance requires an explicit local Docker socket");
    const binaryHash = createHash("sha256");
    for await (const chunk of createReadStream(this.binary)) binaryHash.update(chunk);
    this.runtimeTuple = {
      sourceCommit: await command("git", ["rev-parse", "HEAD"]),
      binaryDigest: `sha256:${binaryHash.digest("hex")}`,
      kernel: release(),
      terraformVersion: JSON.parse(await command(this.terraform, ["version", "-json"])).terraform_version,
      providerLockDigest: `sha256:${createHash("sha256").update(await readFile(resolve(`templates/${this.platform}/.terraform.lock.hcl`))).digest("hex")}`,
    };
    this.engine = JSON.parse(await this.cli("version", "--format", "{{json .Server}}"));
    assert(this.engine.Components?.some(c => c.Name === "Engine" && c.Details?.Os === "linux"), "real Linux Docker Engine required (not Podman)");
    this.directory = await mkdtemp(join(tmpdir(), `${this.prefix}-`));
    this.runnerImage = await runnerImage(this.platform);
    await this.cli("pull", serverImage);
    await this.cli("pull", this.runnerImage);
    await this.startGitlab();
    await this.seed();
    this.oidc = await issuer(this.directory);
    this.port = await freePort();
    this.url = `http://127.0.0.1:${this.port}`;
    this.config = {
      version: 1, storage: { data_dir: join(this.directory, "data") },
      lifecycle: { executor: "exec", max_workers: 8, recovery_reserve: 2, cgroup_root: process.env.SHAULA_TEST_CGROUP },
      http: { listen: `127.0.0.1:${this.port}`, bindings_server_key: randomBytes(32).toString("hex"),
        authorization: [{ issuer: this.oidc.url, subject: "fixture", scopes: scopes.split(" ") }] },
      execution: { engines: { terraform: { executable: this.terraform } }, operation_timeout_secs: 120 },
      runner: { max_lifetime_secs: 900 },
    };
    await this.startDaemon();
    await this.publish();
  }

  // Published on the Docker bridge gateway: both this host (the daemon) and
  // Generation containers on the default bridge reach the same external_url,
  // so clone URLs in job payloads resolve inside the runner too.
  async gitlabPublish(gateway) {
    const port = await freePort(gateway);
    return { port, publish: `${gateway}:${port}:${port}` };
  }

  async startGitlab() {
    const bridge = JSON.parse(await this.cli("network", "inspect", "bridge"))[0];
    const gateway = bridge.IPAM.Config[0].Gateway;
    assert.match(gateway, /^\d+\.\d+\.\d+\.\d+$/);
    const { port, publish } = await this.gitlabPublish(gateway);
    this.instance = `http://${gateway}:${port}`;
    const omnibus = [
      `external_url '${this.instance}'`, "puma['worker_processes'] = 0", "sidekiq['concurrency'] = 5",
      "prometheus_monitoring['enable'] = false", "gitlab_sshd['enable'] = false", "registry['enable'] = false",
      "gitlab_kas['enable'] = false", "letsencrypt['enable'] = false",
    ].join("; ");
    this.server = await this.cli("create", "--name", this.prefix, "--publish", publish,
      "--shm-size", "256m", "-e", `GITLAB_ROOT_PASSWORD=${randomBytes(24).toString("base64url")}`,
      "-e", `GITLAB_OMNIBUS_CONFIG=${omnibus}`, serverImage);
    assert.match(this.server, /^[a-f0-9]{64}$/);
    this.containers.add(this.server);
    await this.cli("start", this.server);
    await until("disposable GitLab API ready", async () => {
      try { return (await fetch(`${this.instance}/api/v4/version`, { signal: AbortSignal.timeout(5000) })).status === 401; }
      catch { return false; }
    }, 900_000);
    const output = await this.cli("exec", this.server, "gitlab-rails", "runner",
      "t = User.find_by_username('root').personal_access_tokens.create!(name: 'fixture', " +
      "scopes: [:api, :admin_mode], expires_at: 2.days.from_now); print t.token");
    this.adminToken = output.split("\n").at(-1);
    assert.match(this.adminToken, /^glpat-/, "generated admin token format");
    await until("authenticated GitLab API", async () => {
      try { return (await fetch(`${this.instance}/api/v4/version`, { headers: { "PRIVATE-TOKEN": this.adminToken } })).ok; }
      catch { return false; }
    });
  }

  // One private group and project; the operator token is a group Owner
  // access token with exactly the spec 0043 scopes.
  async seed() {
    this.group = await this.gitlab("/groups", { method: "POST", expected: 201,
      body: { name: this.prefix, path: this.prefix, visibility: "private" } });
    this.project = await this.gitlab("/projects", { method: "POST", expected: 201,
      body: { name: "app", namespace_id: this.group.id, visibility: "private", initialize_with_readme: true } });
    const expires = new Date(Date.now() + 2 * 86_400_000).toISOString().slice(0, 10);
    this.token = (await this.gitlab(`/groups/${this.group.id}/access_tokens`, { method: "POST", expected: 201,
      body: { name: "shaula", scopes: ["create_runner", "manage_runner", "read_api"], access_level: 50, expires_at: expires } })).token;
    assert.match(this.token, /^glpat-/);
  }

  async startDaemon() {
    const path = join(this.directory, "config.json");
    await writeFile(path, JSON.stringify(this.config), { mode: 0o600 });
    const log = await open(join(this.directory, "daemon.log"), "a", 0o600);
    this.child = spawn(this.binary, ["serve", "--config", path], { stdio: ["ignore", log.fd, log.fd], env: {
      ...process.env, SHAULA_OIDC_PROVIDER: this.oidc.url, SHAULA_OIDC_CLIENT_ID: "fixture", SHAULA_OIDC_CLIENT_SECRET: "fixture-only",
      SHAULA_OIDC_PUBLIC_URL: "https://shaula.example.invalid", SHAULA_OIDC_API_AUDIENCE: "shaula-api", SHAULA_OIDC_CA_CERT: this.oidc.certificate,
    } });
    this.child.on("error", () => { this.spawnFailed = true; });
    await log.close();
    await until("shaula serve ready", async () => {
      assert(!this.spawnFailed && this.child.exitCode === null && this.child.signalCode === null, "shaula exited; inspect protected fixture log");
      try { return (await fetch(`${this.url}/readyz`, { headers: { authorization: `Bearer ${this.oidc.token}` }, signal: AbortSignal.timeout(2000) })).ok; }
      catch { return false; }
    });
  }
  async stopDaemon() {
    if (this.child && this.child.exitCode === null && this.child.signalCode === null) {
      const stopped = new Promise(done => this.child.once("exit", done));
      this.child.kill("SIGKILL");
      await stopped;
    }
  }

  bindings() { return { docker_host: this.socket, runner_backend: "gitlab" }; }

  async publish() {
    const archive = join(this.directory, `${this.platform}.tar.gz`);
    await command("tar", ["-czf", archive, "-C", resolve(`templates/${this.platform}`), "profile.yaml", "main.tf", ".terraform.lock.hcl", "runtime-policy.md", "schemas"]);
    const bytes = await readFile(archive);
    this.digest = `sha256:${createHash("sha256").update(bytes).digest("hex")}`;
    const response = await fetch(`${this.url}/api/v1/template-artifacts/${this.digest}`, { method: "PUT",
      headers: { authorization: `Bearer ${this.oidc.token}` }, body: bytes, signal: AbortSignal.timeout(30_000) });
    assert.equal(response.status, 201, "artifact publication");
    await this.api("/template-profiles/gitlab", "PUT", {
      artifact_digest: this.digest, engine_ref: "terraform", fleet_input_policy: {},
      bindings: this.bindings(),
    }, 202, { "if-none-match": "*" });
    // The auth profile and Fleets go through the actual CLI (docs/cli.md).
    await this.shaula("auth-profiles", "create", "gitlab", { kind: "gitlab_token", instance_url: this.endpoint,
      scope: { kind: "group", path: this.group.full_path }, token: this.token });
    for (const kind of ["template-profiles", "github-auth-profiles"]) {
      await until(`${kind} Active`, async () => {
        const profile = await this.api(`/${kind}/gitlab`);
        assert(!["Rejected", "Unsupported"].includes(profile.status), `${kind}: rejected`);
        return profile.activeRevision === 1;
      });
    }
  }

  async createFleet(key, min = 0, max = 1) {
    this.fleets.add(key);
    await this.shaula("fleets", "create", key, { kind: "gitlab", gitlab: {
      instance_url: this.endpoint, scope: { kind: "group", path: this.group.full_path },
      auth_profile_ref: "gitlab", runner_name_prefix: `${key}-`, tags: [key],
    }, capacity: { min_runners: min, max_runners: max }, template_profile_ref: "gitlab", template_inputs: {} });
  }

  /** `shaula <kind> <action> <key> --file` with a 0600 request file removed after use. */
  async shaula(kind, action, key, body) {
    const credential = join(this.directory, "cli-credential");
    await writeFile(credential, this.oidc.token, { mode: 0o600 });
    const file = join(this.directory, `${kind}-${key}.json`);
    await writeFile(file, JSON.stringify(body), { mode: 0o600, flag: "wx" });
    try {
      const output = JSON.parse(await command(this.binary, ["--server", this.url, "--allow-loopback-http",
        "--credential-kind", "oidc", "--token-file", credential, "--output", "json", kind, action, key, "--file", file]));
      assert.equal(output.error, null, `shaula ${kind} ${action}: accepted`);
      return output;
    } finally {
      await rm(file, { force: true });
    }
  }

  async retireFleet(key) {
    const current = await fetch(`${this.url}/api/v1/fleets/${key}`, {
      headers: { authorization: `Bearer ${this.oidc.token}` }, signal: AbortSignal.timeout(5000) });
    assert.equal(current.status, 200);
    const version = current.headers.get("shaula-resource-version") || current.headers.get("etag");
    await current.body?.cancel();
    assert(version, "Fleet retirement requires its exact current version");
    await this.api(`/fleets/${key}`, "DELETE", undefined, 202, { "if-match": version });
    await until(`${key}: Fleet tombstone`, async () => {
      const response = await fetch(`${this.url}/api/v1/fleets/${key}`, {
        headers: { authorization: `Bearer ${this.oidc.token}` }, signal: AbortSignal.timeout(5000) });
      await response.body?.cancel();
      return response.status === 410;
    }, 180_000);
  }

  /** Every registration in the group scope, including paused ones. */
  async runners() {
    return this.gitlab(`/groups/${this.group.id}/runners?type=group_type&per_page=100`);
  }

  /** Commits the CI file without a push pipeline, so demand is exactly the trigger. */
  async commitCi(content) {
    await this.gitlab(`/projects/${this.project.id}/repository/commits`, { method: "POST", expected: 201, body: {
      branch: "main", commit_message: "lifecycle acceptance [skip ci]",
      actions: [{ action: "create", file_path: ".gitlab-ci.yml", content }],
    } });
  }

  // Platform surface used by run.mjs; kubernetes.mjs overrides each method.
  /** The Fleet's single running Generation container, or false while pending. */
  async runningGeneration(key, rows) {
    const ids = await this.cli("ps", "-a", "--no-trunc", "-q", "--filter", `label=shaula.fleet=${key}`);
    if (!ids) return false;
    assert.equal(ids.split("\n").length, 1, "one matching job must produce exactly one container");
    this.containers.add(ids);
    const [container] = JSON.parse(await this.cli("inspect", ids));
    if (!container.State.Running) return false;
    return { id: ids, generation: rows.items[0].id, volumes: container.Mounts.filter(m => m.Type === "volume").map(m => m.Name) };
  }
  async assertCreated(observed) {
    assert.equal(observed.volumes.length, 2, "exactly the image's two anonymous volumes");
    const [container] = JSON.parse(await this.cli("inspect", observed.id));
    assert.equal(container.Config.User, "gitlab-runner");
    assert.equal(container.HostConfig.RestartPolicy.Name, "no");
    return "one Generation, official image as gitlab-runner, no restart, two anonymous volumes";
  }
  /** Reads the delivered credential from inside the domain, for the leak scan only. */
  deliveredConfig(observed) { return this.cli("exec", observed.id, "cat", "/home/gitlab-runner/config.toml"); }
  /** [label, body, shaulaOwned] runtime surfaces for the leak scan. */
  async runtimeSurfaces(observed) {
    return [["container argv/environment/metadata", await this.cli("inspect", observed.id), true],
      ["runner log", await this.logs(observed.id), false]];
  }
  async assertDestroyed(observed) {
    assert(!(await this.cli("ps", "-a", "-q", "--no-trunc")).split("\n").includes(observed.id), "container remains");
    const volumes = (await this.cli("volume", "ls", "-q")).split("\n");
    assert(observed.volumes.every(v => !volumes.includes(v)), "credential volume remains");
    this.containers.delete(observed.id);
    return "registration deleted after pause → recheck; container and volumes destroyed";
  }
  /** Terraform state body as scanned; Docker state never holds the token. */
  protectedState(body) { return body; }

  async close() {
    await this.stopDaemon();
    await fenceFixture(this);
    // Only resources bearing this fixture's exact random Fleet keys, plus the
    // GitLab server ID. Teardown is never evidence of Shaula cleanup.
    for (const key of this.fleets) {
      const ids = await this.cli("ps", "-a", "--no-trunc", "-q", "--filter", `label=shaula.fleet=${key}`);
      for (const id of ids.split("\n").filter(Boolean)) this.containers.add(id);
    }
    const remaining = new Set((await this.cli("ps", "-a", "--no-trunc", "-q")).split("\n"));
    for (const id of [...this.containers].reverse()) {
      if (remaining.has(id)) await this.cli("rm", "-f", "-v", id);
    }
    await this.oidc?.close();
  }
}
