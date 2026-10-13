// G5 (spec 0043 §4.4, §5): a registration token can serve more than one
// process, so (a) a template whose runner may restart is refused before any
// resource exists, and (b) after the claim and pause a second process with
// the same delivered config.toml cannot acquire another compatible job.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { cp, mkdir, readFile, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { command, until } from "../forgejo-lifecycle/support.mjs";
import { containers, diagnosticCodes, generations, job, jobSucceeded, named, pipeline, reclaimed,
  retired, runningGeneration, steady } from "./scenario-support.mjs";

const marker = "SHAULA_GITLAB_TOKEN_REUSE";
const SECOND_WAIT_SECS = 30;

/** The bundled Docker template with only the runner's restart policy changed. */
async function publishRestarting(fixture) {
  const source = resolve("templates/docker");
  const dir = join(fixture.directory, "restart-template");
  await mkdir(dir, { mode: 0o700 });
  for (const file of ["profile.yaml", ".terraform.lock.hcl", "runtime-policy.md"]) await cp(join(source, file), join(dir, file));
  await cp(join(source, "schemas"), join(dir, "schemas"), { recursive: true });
  const main = await readFile(join(source, "main.tf"), "utf8");
  const changed = main.replace('restart  = "no"', 'restart  = "always"');
  assert.notEqual(changed, main, "bundled template pins restart = \"no\"");
  await writeFile(join(dir, "main.tf"), changed, { mode: 0o600 });
  const archive = join(fixture.directory, "restart.tar.gz");
  await command("tar", ["-czf", archive, "-C", dir, "profile.yaml", "main.tf", ".terraform.lock.hcl", "runtime-policy.md", "schemas"]);
  const bytes = await readFile(archive);
  const digest = `sha256:${createHash("sha256").update(bytes).digest("hex")}`;
  const response = await fetch(`${fixture.url}/api/v1/template-artifacts/${digest}`, { method: "PUT",
    headers: { authorization: `Bearer ${fixture.oidc.token}` }, body: bytes, signal: AbortSignal.timeout(30_000) });
  assert.equal(response.status, 201, "artifact publication");
  await fixture.api("/template-profiles/gitlab-restart", "PUT", {
    artifact_digest: digest, engine_ref: "terraform", fleet_input_policy: {},
    bindings: { docker_host: fixture.socket, runner_backend: "gitlab" },
  }, 202, { "if-none-match": "*" });
  return until("restart template validated", async () => {
    const profile = await fixture.api("/template-profiles/gitlab-restart");
    if (["Rejected", "Unsupported"].includes(profile.status)) return "rejected at publication";
    return profile.activeRevision === 1 ? "accepted at publication (HCL is not evaluated statically)" : false;
  });
}

async function restartPolicy(fixture, checks) {
  const publication = await publishRestarting(fixture);
  checks.restartPublication = publication;
  if (publication.startsWith("rejected")) return;
  const key = `${fixture.prefix}-restart`;
  fixture.fleets.add(key);
  await fixture.shaula("fleets", "create", key, { kind: "gitlab", gitlab: {
    instance_url: fixture.endpoint, scope: { kind: "group", path: fixture.group.full_path },
    auth_profile_ref: "gitlab", runner_name_prefix: `${key}-`, tags: [key],
  }, capacity: { min_runners: 1, max_runners: 1 }, template_profile_ref: "gitlab-restart", template_inputs: {} });
  const noRunner = async () => {
    assert.deepEqual(await containers(fixture, `shaula.fleet=${key}`), [], "a restartable runner resource was created");
    const rows = await generations(fixture, key);
    assert(rows.every(g => !["WaitingOnline", "Idle", "Busy", "Quarantined"].includes(g.state)), "restartable runner admitted");
    return rows;
  };
  const refused = await until(`${key}: saved-plan admission refuses the restart policy`, async () =>
    (await noRunner()).find(g => ["CleanupRequired", "Retiring", "DestroyPending", "Destroying", "Destroyed"].includes(g.state)), 300_000);
  await steady(noRunner, 20_000);
  checks.restartRefusal = { codes: await diagnosticCodes(fixture, "generations", refused.id) };
  // The pre-registered (never bootstrapped) registration is removed with it.
  await retired(fixture, key);
}

/** Second `run-single` on the Generation's own config.toml; stdio discarded (it logs the short token). */
function secondProcess(fixture, container, runnerName) {
  return new Promise((done, fail) => {
    const started = Date.now();
    const child = spawn(fixture.docker, ["--host", fixture.socket, "exec", container, "/usr/bin/gitlab-runner", "run-single",
      "--config", "/home/gitlab-runner/config.toml", "--runner", runnerName, "--max-builds", "1",
      "--wait-timeout", String(SECOND_WAIT_SECS)], { stdio: "ignore" });
    child.on("error", () => fail(new Error("second runner process failed to start")));
    child.on("exit", code => done({ code, seconds: Math.round((Date.now() - started) / 1000) }));
  });
}

async function tokenReuse(fixture, checks) {
  const key = `${fixture.prefix}-token`;
  const script = `  tags: [${key}]\n  script:\n    - echo "${marker} job=$CI_JOB_ID runner=$CI_RUNNER_ID"\n    - sleep 150\n`;
  await fixture.commitCi(`first:\n${script}second:\n${script}`);
  await fixture.createFleet(key, 0, 1);
  const jobs = await pipeline(fixture);
  assert.equal(jobs.length, 2);
  const observed = await runningGeneration(fixture, key, "Busy");
  const [claimed, other] = await until(`${key}: one job claimed`, async () => {
    const current = await Promise.all(jobs.map(j => job(fixture, j.id)));
    const mine = current.find(j => j.runner?.id === observed.runner);
    return mine ? [mine, current.find(j => j.id !== mine.id)] : false;
  });
  await until(`${key}: registration paused after the claim`, async () => (await fixture.gitlab(`/runners/${observed.runner}`)).paused, 120_000);
  assert.equal((await job(fixture, other.id)).status, "pending", "max_runners=1 leaves the second job pending");

  // A second token holder polls for the full wait timeout while paused.
  const second = secondProcess(fixture, observed.container, observed.generation.runner_name);
  let finished;
  second.then(result => { finished = result; }, () => { finished = { failed: true }; });
  let polls = 0;
  while (!finished) {
    const current = await job(fixture, other.id);
    assert.equal(current.status, "pending", "paused token acquired a second job");
    assert.equal(current.runner, null);
    polls++;
    await delay(2000);
  }
  const result = await second;
  assert(result.seconds >= SECOND_WAIT_SECS - 5, "second process exited before polling for its wait timeout");
  assert.equal((await job(fixture, claimed.id)).status, "running", "the original job is unaffected");
  assert.equal((await job(fixture, other.id)).status, "pending");
  checks.tokenReuse = { secondProcessExit: result.code, secondProcessSeconds: result.seconds, pendingPolls: polls };

  // The pending job is served only by a new Generation's own registration.
  const first = await jobSucceeded(fixture, claimed.id);
  await reclaimed(fixture, key, observed);
  const next = await runningGeneration(fixture, key, "Busy", { ignore: [observed.generation.id] });
  assert.notEqual(next.runner, observed.runner);
  await until(`${key}: second job claimed by the new registration`, async () => (await job(fixture, other.id)).runner?.id === next.runner);
  const served = await jobSucceeded(fixture, other.id);
  await reclaimed(fixture, key, next);
  assert.equal((await named(fixture, observed.generation.runner_name)).length, 0);
  await retired(fixture, key);
  checks.tokenReuseConverged = { firstJob: first.status, secondJob: served.status, generations: 2 };
}

export async function tokenSuite(fixture, checks, progress) {
  progress.phase = "restart-policy";
  console.log("Running restart-policy");
  await restartPolicy(fixture, checks);
  progress.phase = "token-reuse";
  console.log("Running token-reuse");
  await tokenReuse(fixture, checks);
}
