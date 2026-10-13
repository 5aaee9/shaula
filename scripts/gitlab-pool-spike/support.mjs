// Administrator-run, disposable local experiment; never targets an existing GitLab.
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { randomBytes, randomUUID } from "node:crypto";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";
import { setTimeout as delay } from "node:timers/promises";

const execute = promisify(execFile);
export const serverImage = "docker.io/gitlab/gitlab-ce:19.4.2-ce.0";
export const runnerImage = "docker.io/gitlab/gitlab-runner:v19.4.1";

export async function until(description, predicate, timeout = 120_000, interval = 1_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const value = await predicate();
    if (value) return value;
    await delay(interval);
  }
  throw new Error(`timed out: ${description}`);
}

async function freePort() {
  const server = createServer();
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const port = server.address().port;
  await new Promise((resolve) => server.close(resolve));
  return port;
}

export class Fixture {
  engine = process.env.CONTAINER_ENGINE ?? "docker";
  prefix = `shaula-gl-${randomUUID().slice(0, 8)}`;
  containers = new Set();
  volumes = new Set();
  network;
  adminToken;

  async command(...args) {
    try {
      const { stdout, stderr } = await execute(this.engine, args, {
        timeout: 300_000,
        maxBuffer: 16 * 1024 * 1024,
      });
      return (args[0] === "logs" ? stdout + stderr : stdout).trim();
    } catch (error) {
      // CLI output can contain bootstrap/admin credentials. Never echo it.
      throw new Error(`${this.engine} ${args[0]} failed (exit ${error.code ?? "unknown"})`);
    }
  }

  async create(args, image, command = []) {
    const id = await this.command("create", ...args, image, ...command);
    assert.match(id, /^[a-f0-9]{64}$/);
    this.containers.add(id);
    const [container] = JSON.parse(await this.command("inspect", id));
    for (const mount of container.Mounts ?? []) {
      if (mount.Type === "volume") this.volumes.add(mount.Name);
    }
    return id;
  }

  async start() {
    this.directory = await mkdtemp(join(tmpdir(), `${this.prefix}-`));
    this.network = this.prefix;
    await this.command("network", "create", this.network);
    const port = await freePort();
    this.url = `http://127.0.0.1:${port}`;
    // Runners resolve the server by container name, so clone URLs derived from
    // external_url work inside the private network; the host uses the published port.
    this.internalUrl = `http://${this.prefix}-server:8929`;
    const omnibus = [
      `external_url '${this.internalUrl}'`,
      "puma['worker_processes'] = 0",
      "sidekiq['concurrency'] = 5",
      "prometheus_monitoring['enable'] = false",
      "gitlab_sshd['enable'] = false",
      "registry['enable'] = false",
      "gitlab_kas['enable'] = false",
      "letsencrypt['enable'] = false",
    ].join("; ");
    this.server = await this.create(
      [
        "--name",
        `${this.prefix}-server`,
        "--network",
        this.network,
        "--publish",
        `127.0.0.1:${port}:8929`,
        "--shm-size",
        "256m",
        "-e",
        `GITLAB_ROOT_PASSWORD=${randomBytes(24).toString("base64url")}`,
        "-e",
        `GITLAB_OMNIBUS_CONFIG=${omnibus}`,
      ],
      serverImage,
    );
    await this.command("start", this.server);
    await until(
      "disposable GitLab API ready",
      async () => {
        try {
          const response = await fetch(`${this.url}/api/v4/version`, {
            signal: AbortSignal.timeout(5_000),
          });
          return response.status === 401;
        } catch {
          return false;
        }
      },
      900_000,
      5_000,
    );
    const output = await this.command(
      "exec",
      this.server,
      "gitlab-rails",
      "runner",
      "t = User.find_by_username('root').personal_access_tokens.create!(name: 'experiment', " +
        "scopes: [:api, :admin_mode], expires_at: 2.days.from_now); print t.token",
    );
    this.adminToken = output.split("\n").at(-1);
    assert.match(this.adminToken, /^glpat-/, "generated admin token format");
    // Sidekiq may still be warming up; wait until an authenticated read succeeds.
    await until("authenticated API", async () => {
      const response = await fetch(`${this.url}/api/v4/version`, {
        headers: { "PRIVATE-TOKEN": this.adminToken },
      });
      return response.ok;
    });
  }

  async api(path, { method = "GET", body, token = this.adminToken, expected = 200 } = {}) {
    const response = await fetch(`${this.url}/api/v4${path}`, {
      method,
      headers: { "PRIVATE-TOKEN": token, "Content-Type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
      signal: AbortSignal.timeout(15_000),
      redirect: "error",
    });
    const status = response.status;
    if (expected !== null) assert.equal(status, expected, `${method} ${path}: HTTP ${status}`);
    const text = await response.text();
    return { status, body: text ? JSON.parse(text) : null };
  }

  async graphql(query, token) {
    const response = await fetch(`${this.url}/api/graphql`, {
      method: "POST",
      headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
      body: JSON.stringify({ query }),
      signal: AbortSignal.timeout(15_000),
      redirect: "error",
    });
    assert.equal(response.status, 200, "GraphQL HTTP status");
    const payload = await response.json();
    assert.equal(payload.errors, undefined, "GraphQL errors");
    return payload.data;
  }

  /** Starts `run-single --max-builds 1`; the token reaches the container only as a copied file. */
  async runner(registration, name, waitTimeout) {
    const id = await this.create(
      ["--name", `${this.prefix}-${name}`, "--network", this.network, "--restart", "no"],
      runnerImage,
      [
        "run-single",
        "--config",
        "/etc/gitlab-runner/config.toml",
        "--runner",
        name,
        "--max-builds",
        "1",
        "--wait-timeout",
        String(waitTimeout),
      ],
    );
    const config =
      "concurrent = 1\ncheck_interval = 2\n[[runners]]\n" +
      `  name = "${name}"\n  url = "${this.internalUrl}"\n` +
      `  token = "${registration.token}"\n  executor = "shell"\n`;
    const path = join(this.directory, `${name}.toml`);
    await writeFile(path, config, { mode: 0o600, flag: "wx" });
    await this.command("cp", path, `${id}:/etc/gitlab-runner/config.toml`);
    await rm(path);
    await this.command("start", id);
    return id;
  }

  async inspect(id) {
    return JSON.parse(await this.command("inspect", id))[0];
  }

  async exited(id, timeout = 180_000) {
    return until(`${id.slice(0, 12)} exit`, async () => {
      const { State } = await this.inspect(id);
      return State.Running ? false : State;
    }, timeout);
  }

  async logs(id) {
    return this.command("logs", id);
  }

  async close() {
    // Only IDs created by this fixture, never a name/prefix search or prune.
    const failures = [];
    for (const id of [...this.containers].reverse()) {
      try {
        await this.command("rm", "--force", "--volumes", id);
        this.containers.delete(id);
      } catch {
        failures.push(id);
      }
    }
    const existing = (await this.command("volume", "ls", "--format", "{{.Name}}")).split("\n");
    for (const name of [...this.volumes].filter((volume) => existing.includes(volume))) {
      await this.command("volume", "rm", name).catch(() => failures.push(name));
    }
    this.volumes.clear();
    if (this.network) await this.command("network", "rm", this.network).catch(() => failures.push(this.network));
    if (this.directory) await rm(this.directory, { recursive: true, force: true });
    assert.equal(failures.length, 0, `fixture teardown failed for: ${failures.join(",")}`);
  }
}
