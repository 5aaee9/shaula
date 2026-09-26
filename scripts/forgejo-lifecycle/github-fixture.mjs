// Reuses process/HTTP helpers, but never starts a Forgejo server or substitutes
// GitHub responses. Run on the host already entrusted with the GitHub App key.
import assert from "node:assert/strict";
import { createHash, randomBytes } from "node:crypto";
import { createReadStream } from "node:fs";
import { lstat, mkdtemp, readFile } from "node:fs/promises";
import { tmpdir, release } from "node:os";
import { join, resolve } from "node:path";
import { Fixture } from "./fixture.mjs";
import { issuer, scopes } from "./oidc.mjs";
import { command, freePort, until } from "./support.mjs";

export class GithubFixture extends Fixture {
  async start() {
    assert(process.platform === "linux", "Linux acceptance host required");
    assert(this.socket.startsWith("unix:///"), "explicit local Docker socket required");
    const source = process.env.SHAULA_ACCEPTANCE_GITHUB_PROFILE;
    assert(source && source.startsWith("/"), "protected existing GitHub App profile file required");
    const metadata = await lstat(source);
    assert(metadata.isFile() && !metadata.isSymbolicLink() && metadata.uid === process.geteuid()
      && (metadata.mode & 0o077) === 0 && metadata.size <= 65536, "profile file must be private and owned by the acceptance operator");
    const profile = JSON.parse(await readFile(source, "utf8"));
    assert.equal(profile.kind, "github_app");
    assert.equal(profile.schema_version, 2);
    // This test may only target the existing repository, regardless of the
    // broader production profile's permitted organizations.
    profile.target_policy = [{ kind: "account_repositories", account_kind: "user", owner: "5aaee9" }];
    this.directory = await mkdtemp(join(tmpdir(), `${this.prefix}-github-`));
    const hash = createHash("sha256");
    for await (const chunk of createReadStream(this.binary)) hash.update(chunk);
    this.runtimeTuple = {
      sourceCommit: await command("git", ["rev-parse", "HEAD"]),
      binaryDigest: `sha256:${hash.digest("hex")}`, kernel: release(), containment: "linux-cgroup-v2",
      terraformVersion: JSON.parse(await command(this.terraform, ["version", "-json"])).terraform_version,
      providerLockDigest: `sha256:${createHash("sha256").update(await readFile(resolve("templates/docker/.terraform.lock.hcl"))).digest("hex")}`,
      dockerVersion: JSON.parse(await this.cli("version", "--format", "{{json .Server}}" )).Version,
    };
    this.oidc = await issuer(this.directory);
    this.port = await freePort();
    this.url = `http://127.0.0.1:${this.port}`;
    this.config = {
      version: 1, storage: { data_dir: join(this.directory, "data") },
      lifecycle: { executor: "exec", max_workers: 4, recovery_reserve: 1, cgroup_root: process.env.SHAULA_TEST_CGROUP },
      http: { listen: `127.0.0.1:${this.port}`, bindings_server_key: randomBytes(32).toString("hex"),
        authorization: [{ issuer: this.oidc.url, subject: "fixture", scopes: scopes.split(" ") }] },
      execution: { engines: { terraform: { executable: this.terraform } }, operation_timeout_secs: 180 },
      runner: { max_lifetime_secs: 900 },
    };
    await this.startDaemon();
    const archive = join(this.directory, "docker.tar.gz");
    await command("tar", ["-czf", archive, "-C", resolve("templates/docker"), "profile.yaml", "main.tf", ".terraform.lock.hcl", "runtime-policy.md", "schemas"]);
    const bytes = await readFile(archive);
    this.digest = `sha256:${createHash("sha256").update(bytes).digest("hex")}`;
    const response = await fetch(`${this.url}/api/v1/template-artifacts/${this.digest}`, { method: "PUT",
      headers: { authorization: `Bearer ${this.oidc.token}` }, body: bytes, signal: AbortSignal.timeout(30_000) });
    assert.equal(response.status, 201);
    await this.api("/template-profiles/github", "PUT", { artifact_digest: this.digest, engine_ref: "terraform",
      bindings: { docker_host: this.socket, runner_backend: "github" }, fleet_input_policy: {} }, 202, { "if-none-match": "*" });
    await this.api("/github-auth-profiles/github", "PUT", profile, 202, { "if-none-match": "*" });
    profile.private_key = undefined;
    for (const kind of ["template-profiles", "github-auth-profiles"]) {
      await until(`${kind} Active`, async () => {
        const value = await this.api(`/${kind}/github`);
        assert(!["Rejected", "Unsupported"].includes(value.status), `${kind}: rejected`);
        return value.activeRevision === 1;
      });
    }
  }

  async createFleet(key) {
    this.fleets.add(key);
    await this.api(`/fleets/${key}`, "PUT", { kind: "github", github: {
      target: { kind: "repository", owner: "5aaee9", repository: "shaula" },
      auth_profile_ref: "github", scale_set_name: key, runner_group: "Default", labels: [key],
    }, capacity: { min_runners: 0, max_runners: 1 }, template_profile_ref: "github", template_inputs: {} },
    202, { "if-none-match": "*" });
    await until("GitHub listener is ready before Browser dispatch", async () =>
      (await this.api(`/fleets/${key}/status`)).phase === "Ready");
  }
}
