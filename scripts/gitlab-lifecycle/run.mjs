#!/usr/bin/env node
// Actual shaula serve + real GitLab CE + real Terraform/Docker (spec 0043 §10).
// One demand-driven Generation: anchor → Create → claim → pause → finish →
// retire → Destroy, then Fleet retirement removes the anchor.
import assert from "node:assert/strict";
import { writeFile } from "node:fs/promises";
import { join } from "node:path";
import { GitlabFixture, serverImage } from "./fixture.mjs";
import { leakScan } from "./leaks.mjs";
import { until } from "../forgejo-lifecycle/support.mjs";

process.umask(0o077);
const fixture = new GitlabFixture();
const report = { kind: "shaula-gitlab-lifecycle/v1", platform: "docker", serverImage, checks: {}, passed: false };
const marker = "SHAULA_GITLAB_TASK_EXECUTED";
let phase = "setup";
let completed = false;
try {
  await fixture.start();
  Object.assign(report, { runtimeTuple: fixture.runtimeTuple, engineVersion: fixture.engine.Version,
    artifactDigest: fixture.digest, runnerImage: fixture.runnerImage, gitlabVersion: (await fixture.gitlab("/version")).version });
  const key = `${fixture.prefix}-demand`;
  await fixture.commitCi(`match:\n  tags: [${key}]\n  script:\n    - echo "${marker} job=$CI_JOB_ID runner=$CI_RUNNER_ID"\n    - sleep 30\n` +
    `mismatch:\n  tags: [${key}, gpu]\n  script: [echo must-stay-pending]\n`);

  phase = "anchor";
  await fixture.createFleet(key);
  const anchor = await until(`${key}: paused anchor registration`, async () =>
    (await fixture.runners()).find(r => r.description === `${key}-anchor-r1`), 180_000);
  assert.equal(anchor.paused, true, "anchor never serves jobs");
  const idle = await fixture.api(`/fleets/${key}/status`);
  assert.equal(idle.capacity.occupancy, 0, "idle anchor demand creates nothing");
  report.checks.anchor = "paused, tokenless; zero demand creates no Generation";

  phase = "create";
  const pipeline = await fixture.gitlab(`/projects/${fixture.project.id}/pipeline?ref=main`, { method: "POST", expected: 201 });
  const observed = await until(`${key}: Generation container running`, async () => {
    const rows = await fixture.api(`/generations?fleet_key=${key}`);
    assert(rows.items.every(g => !["Quarantined", "CleanupRequired"].includes(g.state)), "unexpected quarantine/create failure; inspect protected daemon log");
    const ids = await fixture.cli("ps", "-a", "--no-trunc", "-q", "--filter", `label=shaula.fleet=${key}`);
    if (!ids) return false;
    assert.equal(ids.split("\n").length, 1, "one matching job must produce exactly one container");
    fixture.containers.add(ids);
    const [container] = JSON.parse(await fixture.cli("inspect", ids));
    if (!container.State.Running) return false;
    return { id: ids, generation: rows.items[0].id, volumes: container.Mounts.filter(m => m.Type === "volume").map(m => m.Name) };
  }, 300_000);
  assert.equal(observed.volumes.length, 2, "exactly the image's two anonymous volumes");
  const [container] = JSON.parse(await fixture.cli("inspect", observed.id));
  assert.equal(container.Config.User, "gitlab-runner");
  assert.equal(container.HostConfig.RestartPolicy.Name, "no");
  report.checks.create = "one Generation, official image as gitlab-runner, no restart, two anonymous volumes";

  phase = "claim";
  const jobs = await fixture.gitlab(`/projects/${fixture.project.id}/pipelines/${pipeline.id}/jobs`);
  const match = jobs.find(j => j.name === "match");
  const claimed = await until(`${key}: job claimed by the Generation registration`, async () => {
    const job = await fixture.gitlab(`/projects/${fixture.project.id}/jobs/${match.id}`);
    return job.runner?.id ? job : false;
  }, 300_000);
  const registration = await fixture.gitlab(`/runners/${claimed.runner.id}`);
  assert(registration.description.startsWith(`${key}-`) && registration.description !== anchor.description);
  assert.equal(registration.maintenance_note, `shaula:fleet=${key};role=generation;name=${registration.description}`);
  // Read the delivered credential from inside the domain for the leak scan only.
  const config = await fixture.cli("exec", observed.id, "cat", "/home/gitlab-runner/config.toml");
  const runnerToken = /token = "(glrt-[^"]+)"/.exec(config)?.[1];
  assert(runnerToken, "delivered config.toml carries the registration token");
  await until(`${key}: claimed registration paused`, async () => (await fixture.gitlab(`/runners/${claimed.runner.id}`)).paused, 120_000);
  const secrets = [fixture.token, runnerToken];
  const short = /with token (\S+)/.exec(await fixture.logs(observed.id))?.[1];
  assert(short && short.length >= 6 && runnerToken.includes(short), "runner logs its short token");
  report.leakScanWhileRunning = await leakScan(fixture, key, observed, secrets, true, [short]);
  report.checks.claim = "job.runner is the owned Generation registration; paused while the job runs";

  phase = "finish";
  const finished = await until(`${key}: job success`, async () => {
    const job = await fixture.gitlab(`/projects/${fixture.project.id}/jobs/${match.id}`);
    assert(!["failed", "canceled"].includes(job.status), `job ${job.status}`);
    return job.status === "success" ? job : false;
  }, 300_000);
  const trace = await fixture.gitlab(`/projects/${fixture.project.id}/jobs/${match.id}/trace`);
  assert(trace.includes(`${marker} job=${match.id} runner=${claimed.runner.id}`), "job output names the Generation registration");
  for (const secret of secrets) assert(!trace.includes(secret), "credential leak in job trace");
  const mismatch = await fixture.gitlab(`/projects/${fixture.project.id}/jobs/${jobs.find(j => j.name === "mismatch").id}`);
  assert.equal(mismatch.status, "pending", "an incompatible job is never served");
  // gitlab-runner itself writes "on <runner> <short token>" into every job
  // trace; that GitLab-owned surface is recorded, not asserted (spec 0043 G8).
  report.checks.finish = { job: finished.status, durationSeconds: finished.duration, traceHasTaskOutput: true,
    traceShowsUpstreamShortToken: trace.includes(short) };

  phase = "reclaim";
  await until(`${key}: full reclaim`, async () => (await fixture.api(`/fleets/${key}/status`)).capacity.occupancy === 0, 900_000);
  const generations = await fixture.api(`/generations?fleet_key=${key}`);
  assert.equal(generations.items.length, 1, "no duplicate Generation for one job");
  assert.equal(generations.items[0].state, "Destroyed");
  await fixture.gitlab(`/runners/${claimed.runner.id}`, { expected: 404 });
  assert(!(await fixture.cli("ps", "-a", "-q", "--no-trunc")).split("\n").includes(observed.id), "container remains");
  const volumes = (await fixture.cli("volume", "ls", "-q")).split("\n");
  assert(observed.volumes.every(v => !volumes.includes(v)), "credential volume remains");
  fixture.containers.delete(observed.id);
  report.leakScanAfterDestroy = await leakScan(fixture, key, observed, secrets, false, [short]);
  report.checks.reclaim = "registration deleted after pause → recheck; container and volumes destroyed";

  phase = "retire";
  await fixture.retireFleet(key);
  const leftover = (await fixture.runners()).filter(r => r.description.startsWith(`${key}-`));
  assert.equal(leftover.length, 0, "Fleet retirement leaves no anchor or Generation registration");
  report.checks.retire = "Fleet tombstoned (410); anchor deleted";
  report.passed = completed = true;
} catch (error) {
  report.failedPhase = phase;
  // No bodies, secret-bearing subprocess output or raw server logs in reports.
  report.reason = error.message;
  process.exitCode = 1;
} finally {
  try {
    await fixture.close();
  } catch {
    report.passed = false;
    report.teardownFailed = true;
    process.exitCode = 1;
  }
  report.passed &&= completed;
  if (fixture.directory) {
    await writeFile(join(fixture.directory, "report.json"), JSON.stringify(report, null, 2), { mode: 0o600 });
    console.log(`Private evidence: ${fixture.directory}`);
  }
  if (process.env.SHAULA_GITLAB_REPORT) await writeFile(process.env.SHAULA_GITLAB_REPORT, JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report, null, 2));
}
