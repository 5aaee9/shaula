#!/usr/bin/env node
// Actual shaula serve + real Forgejo + real Terraform. No simulated runtime.
import assert from "node:assert/strict";
import { writeFile } from "node:fs/promises";
import { join } from "node:path";
import { Fixture } from "./fixture.mjs";
import { KubernetesFixture } from "./kubernetes.mjs";
import { labels, staleDemand, lostRegistration } from "./scenarios.mjs";
import { permissions } from "./permissions.mjs";
import { captureCredentials, leakScan } from "./leaks.mjs";
import { expiry } from "./expiry.mjs";
import { until, serverImage, runnerImage } from "./support.mjs";

process.umask(0o077);
assert([undefined, "docker", "kubernetes"].includes(process.env.SHAULA_ACCEPTANCE_BACKEND), "unsupported acceptance backend");
const fixture = process.env.SHAULA_ACCEPTANCE_BACKEND === "kubernetes" ? new KubernetesFixture() : new Fixture();
const report = { kind: "shaula-forgejo-lifecycle/v2", platform: fixture.platform, serverImage, runnerImage, checks: {}, passed: false };
let phase = "setup";
try {
  await fixture.start();
  report.engineVersion = fixture.engine.Version;
  report.artifactDigest = fixture.digest;
  for (const scenario of ["success", "failure", "restart"]) {
    phase = scenario;
    console.log(`Running ${scenario}`);
    const key = `${fixture.prefix}-${scenario}`;
    await fixture.queue(key, scenario === "success" ? 40 : scenario === "restart" ? 20 : 8, scenario === "failure");
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
    let secrets;
    if (scenario === "success") {
      secrets = await captureCredentials(fixture, observed);
      report.leakScanBeforeCompletion = await leakScan(fixture, key, observed, secrets, true);
    }
    if (scenario === "restart") await fixture.restart();
    await until(`${key}: workflow conclusion`, async () => {
      const runs = await fixture.forgejo(`/repos/${fixture.user}/${key}/actions/runs`);
      return runs.workflow_runs.some(run => run.status === (scenario === "failure" ? "failure" : "success"));
    });
    await fixture.reclaimed(key, observed);
    await until(`${key}: exact task result after snapshot disappearance`, async () => {
      const retained = await fixture.api(`/jobs/${job.id}`);
      assert.equal(retained.association_status, "unverified");
      assert.equal(retained.generations.length, 0);
      if (!retained.forgejo.result) return false;
      assert.equal(retained.reported_result, scenario === "failure" ? "failure" : "success");
      assert.equal(retained.forgejo.result.task_id, job.forgejo.task_id);
      assert(retained.forgejo_observations.some(event => event.source === "task_history"));
      assert(retained.forgejo.result.run_url.startsWith(`${fixture.target}/${fixture.user}/${key}/actions/runs/`));
      return retained.observed_status === "completed" && retained.forgejo.in_snapshot === false;
    });
    if (secrets) {
      report.leakScanAfterCompletion = await leakScan(fixture, key, observed, secrets, false);
      report.setupInfo = "not emitted by bundled Forgejo v1 templates";
      report.checks.leakScan = "passed";
    }
    report.checks.forgejoJobsProjection = "exact task results; runner associations remain Unverified";
    report.checks[scenario] = "passed";
  }
  for (const [name, check] of [["labels", labels], ["staleDemand", staleDemand]]) {
    phase = name;
    console.log(`Running ${name}`);
    await check(fixture);
    report.checks[name] = "passed";
  }
  if (fixture.platform === "docker") {
    phase = "permissions";
    console.log(`Running ${phase}`);
    report.permissions = await permissions(fixture);
    report.checks.minimumPermissions = "passed";
  }
  phase = "hard-expiry";
  console.log(`Running ${phase}`);
  await expiry(fixture, report.checks);
  phase = "lost-registration";
  console.log(`Running ${phase}`);
  await lostRegistration(fixture);
  report.checks.lostRegistration = "quarantined; no POST replay or resource Create; occupancy held";
  report.faults = { dockerDelete: fixture.dockerProxy.gate.failures, registrationDelete: fixture.registrationProxy.gate.failures,
    jobsRead: fixture.registrationProxy.gate.jobFailures, registrationResponseLost: fixture.registrationProxy.gate.droppedRegistrations };
  report.passed = true;
} catch (error) {
  report.failedPhase = phase;
  // No bodies, secret-bearing subprocess output or raw server logs in reports.
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
