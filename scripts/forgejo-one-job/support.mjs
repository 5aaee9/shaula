// Administrator-run, disposable local experiment; never targets an existing Forgejo.
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { randomUUID } from "node:crypto";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";
import { setTimeout as delay } from "node:timers/promises";

const execute = promisify(execFile);
export const serverImage = "codeberg.org/forgejo/forgejo:16.0.4";
export const runnerImage = "code.forgejo.org/forgejo/runner:13.1.0";

export async function until(description, predicate, timeout = 90_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const value = await predicate();
    if (value) return value;
    await delay(250);
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
  prefix = `shaula-one-job-${randomUUID().slice(0, 8)}`;
  containers = new Set();
  volumes = new Set();
  user = "one-job-test";
  token;

  async command(...args) {
    try {
      const { stdout, stderr } = await execute(this.engine, args, {
        timeout: 120_000,
        maxBuffer: 8 * 1024 * 1024,
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
    const port = await freePort();
    this.url = `http://127.0.0.1:${port}`;
    this.server = await this.create(
      [
        "--name",
        this.prefix,
        "--network",
        "host",
        "-e",
        "FORGEJO__actions__ENABLED=true",
        "-e",
        "FORGEJO__security__INSTALL_LOCK=true",
        "-e",
        "FORGEJO__service__DISABLE_REGISTRATION=true",
        "-e",
        "FORGEJO__server__DISABLE_SSH=true",
        "-e",
        "FORGEJO__server__HTTP_ADDR=127.0.0.1",
        "-e",
        `FORGEJO__server__HTTP_PORT=${port}`,
        "-e",
        `FORGEJO__server__ROOT_URL=${this.url}/`,
      ],
      serverImage,
    );
    await this.command("start", this.server);
    await until("disposable Forgejo ready", async () => {
      try {
        const response = await fetch(`${this.url}/api/v1/version`, {
          signal: AbortSignal.timeout(2_000),
        });
        return response.ok && (await response.json());
      } catch {
        return false;
      }
    });
    await this.command(
      "exec",
      "--user",
      "git",
      this.server,
      "forgejo",
      "admin",
      "user",
      "create",
      "--username",
      this.user,
      "--random-password",
      "--random-password-length",
      "32",
      "--email",
      "one-job@example.invalid",
      "--admin",
      "--must-change-password=false",
    );
    this.token = await this.command(
      "exec",
      "--user",
      "git",
      this.server,
      "forgejo",
      "admin",
      "user",
      "generate-access-token",
      "--username",
      this.user,
      "--token-name",
      "experiment",
      "--scopes",
      "all",
      "--raw",
    );
    assert(/^[a-f0-9]{40}$/.test(this.token), "generated admin token format");
  }

  async api(path, method = "GET", body, expected = 200) {
    const response = await fetch(`${this.url}/api/v1${path}`, {
      method,
      headers: { Authorization: `Bearer ${this.token}`, "Content-Type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
      signal: AbortSignal.timeout(10_000),
      redirect: "error",
    });
    assert.equal(response.status, expected, `${method} ${path}: unexpected HTTP status`);
    return response.status === 204 || response.status === 404 ? null : response.json();
  }

  async queue(name, label, fail = false) {
    await this.api(
      "/user/repos",
      "POST",
      {
        name,
        private: true,
        auto_init: true,
        default_branch: "main",
      },
      201,
    );
    const workflow = `name: one-job-test\non: [push]\njobs:\n  check:\n    runs-on: [${label}]\n    steps:\n      - run: |\n          echo TASK_EXECUTED\n          sleep 3\n          exit ${fail ? 1 : 0}\n`;
    await this.api(
      `/repos/${this.user}/${name}/contents/.forgejo/workflows/check.yaml`,
      "POST",
      {
        content: Buffer.from(workflow).toString("base64"),
        message: "one-job test",
        branch: "main",
      },
      201,
    );
    await until(`${name}: waiting job`, async () =>
      (await this.jobs(label)).some((job) => job.status === "waiting"),
    );
  }

  async jobs(label) {
    return (
      (await this.api(`/admin/actions/runners/jobs?labels=${encodeURIComponent(label)}`)) ?? []
    );
  }

  async register(name) {
    return this.api(
      "/admin/actions/runners",
      "POST",
      {
        name: `${this.prefix}-${name}`,
        ephemeral: true,
      },
      201,
    );
  }

  async runner(registration, label, url, wait) {
    const args = [
      "/bin/forgejo-runner",
      "one-job",
      "--url",
      url,
      "--uuid",
      registration.uuid,
      "--token-url",
      "file:///data/token",
      "--label",
      `${label}:host`,
    ];
    if (wait) args.push("--wait");
    const id = await this.create(
      [
        "--name",
        `${this.prefix}-runner-${registration.id}`,
        "--network",
        "host",
        "--restart",
        "no",
      ],
      runnerImage,
      args,
    );
    const path = join(this.directory, `${registration.id}.token`);
    // Parent is 0700; copied file must be readable by the image's UID 1000.
    await writeFile(path, registration.token, { mode: 0o644, flag: "wx" });
    await this.command("cp", path, `${id}:/data/token`);
    await rm(path);
    await this.command("start", id);
    return id;
  }

  async state(id) {
    return JSON.parse(await this.command("inspect", id))[0].State;
  }

  async removeStopped(id) {
    assert.equal((await this.state(id)).Running, false, "never reclaim a running container");
    await this.command("rm", "--volumes", id);
    this.containers.delete(id);
    const remaining = await this.command("ps", "--all", "--no-trunc", "--format", "{{.ID}}");
    assert(!remaining.split("\n").includes(id), "container still exists");
  }

  async close() {
    // Only IDs created by this fixture, never a name/prefix search or prune.
    // Failure cases retain their registration until this disposable server is torn down.
    const failures = [];
    for (const id of [...this.containers].reverse()) {
      try {
        await this.command("rm", "--force", "--volumes", id);
        this.containers.delete(id);
      } catch {
        failures.push(id);
      }
    }
    if (this.directory) await rm(this.directory, { recursive: true, force: true });
    assert.equal(failures.length, 0, `fixture teardown failed for IDs: ${failures.join(",")}`);
    if (this.volumes.size) {
      const remaining = (await this.command("volume", "ls", "--format", "{{.Name}}")).split("\n");
      assert(
        [...this.volumes].every((name) => !remaining.includes(name)),
        "fixture volumes remain",
      );
      this.volumes.clear();
    }
  }
}
