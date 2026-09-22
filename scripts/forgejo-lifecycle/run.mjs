#!/usr/bin/env node
// Actual shaula serve + real Forgejo + real Docker/Terraform. No simulated runtime.
import assert from "node:assert/strict";
import { writeFile } from "node:fs/promises";
import { join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { Fixture } from "./fixture.mjs";
import { until, serverImage, runnerImage } from "./support.mjs";

process.umask(0o077);
const fixture = new Fixture();
const report = { kind: "shaula-forgejo-docker-lifecycle/v1", serverImage, runnerImage, checks: {}, passed: false };
let phase = "setup";
try {
  await fixture.start();
  report.engineVersion = fixture.engine.Version;
  report.artifactDigest = fixture.digest;
  for (const scenario of ["success", "failure", "restart"]) {
    phase = scenario;
    console.log(`Running ${scenario}`);
    const key = `${fixture.prefix}-${scenario}`;
    await fixture.queue(key, scenario === "restart" ? 20 : 8, scenario === "failure");
    await fixture.createFleet(key);
    const observed = await fixture.observe(key, "active");
    const job = await until(`${key}: retained running job`, async () => {
      const page = await fixture.api(`/jobs?fleet_key=${key}`);
      return page.items.find(j => j.observed_status === "running");
    });
    assert.equal(job.backend, "forgejo");
    assert.equal(job.scale_set_id, undefined, "Forgejo must not synthesize a Scale Set identity");
    assert.equal(job.association_status, "unverified");
    const detail = await fixture.api(`/jobs/${job.id}`);
    assert.equal(detail.generations.length, 0, "snapshot timing must not create verified runner links");
    if (scenario === "restart") await fixture.restart();
    await until(`${key}: workflow conclusion`, async () => {
      const runs = await fixture.forgejo(`/repos/${fixture.user}/${key}/actions/runs`);
      return runs.workflow_runs.some(run => run.status === (scenario === "failure" ? "failure" : "success"));
    });
    await fixture.reclaimed(key, observed);
    await until(`${key}: no inferred job completion`, async () => {
      const retained = await fixture.api(`/jobs/${job.id}`);
      assert.equal(retained.reported_result, null);
      return retained.observed_status === "unknown" && retained.forgejo.in_snapshot === false;
    });
    report.checks.forgejoJobsProjection = "passed";
    report.checks[scenario] = "passed";
  }

  phase = "idle-expiry";
  console.log(`Running ${phase}`);
  await fixture.restart(15);
  const idleKey = `${fixture.prefix}-idle`;
  await fixture.createFleet(idleKey, 1);
  const idle = await fixture.observe(idleKey, "idle");
  await fixture.capZero(idleKey);
  await fixture.reclaimed(idleKey, idle);
  report.checks.idleExpiry = "passed";

  phase = "busy-expiry-delete-retries";
  console.log(`Running ${phase}`);
  const key = `${fixture.prefix}-busy`;
  await fixture.queue(key, 180);
  await fixture.createFleet(key);
  const busy = await fixture.observe(key, "active");
  fixture.dockerProxy.gate.block = true;
  fixture.registrationProxy.gate.block = true;
  await fixture.capZero(key);
  await until("Docker Destroy failed and remains retryable", () => fixture.dockerProxy.gate.failures > 0);
  assert((await fixture.api(`/fleets/${key}/status`)).capacity.occupancy > 0, "failed Destroy cannot release occupancy");
  assert((await fixture.forgejo("/admin/actions/runners")).some(r => r.id === busy.runner && r.status === "active"), "ordinary drain must preserve a busy registration");
  await fixture.restart();
  fixture.dockerProxy.gate.block = false;
  await until("registration delete is retried after resource destruction", () => fixture.registrationProxy.gate.failures > 0);
  assert(!(await fixture.cli("ps", "-a", "--no-trunc", "-q")).split("\n").includes(busy.id), "hard expiry must destroy the busy resource before registration removal");
  assert((await fixture.api(`/fleets/${key}/status`)).capacity.occupancy > 0, "registration failure retains occupancy");
  assert((await fixture.forgejo("/admin/actions/runners")).some(r => r.id === busy.runner), "registration retry checkpoint must be exercised");
  await fixture.restart();
  await delay(500);
  fixture.registrationProxy.gate.block = false;
  await fixture.reclaimed(key, busy);
  report.checks.busyExpiry = "passed";
  report.checks.destroyRetryAcrossRestart = "passed";
  report.checks.registrationRetryAcrossRestart = "passed";
  report.faults = { dockerDelete: fixture.dockerProxy.gate.failures, registrationDelete: fixture.registrationProxy.gate.failures };
  report.passed = true;
} catch (error) {
  report.failedPhase = phase;
  // Diagnostics remain in the 0700 fixture directory. Never publish a body,
  // secret-bearing subprocess output, or raw server logs in a failure report.
  report.reason = error.message;
  process.exitCode = 1;
} finally {
  try { await fixture.close(); }
  catch { report.passed = false; report.teardownFailed = true; process.exitCode = 1; }
  if (fixture.directory) {
    await writeFile(join(fixture.directory, "report.json"), JSON.stringify(report, null, 2), { mode: 0o600 });
    console.log(`Private evidence: ${fixture.directory}`);
  }
  console.log(JSON.stringify(report, null, 2));
}
