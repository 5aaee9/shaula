#!/usr/bin/env node
// Browser dispatch is deliberately external: the operator selects this exact
// random label and branch in the existing GitHub workflow. No canned CI demand.
import assert from "node:assert/strict";
import { writeFile } from "node:fs/promises";
import { join } from "node:path";
import { GithubFixture } from "./github-fixture.mjs";
import { workers } from "./worker-pressure.mjs";
import { fenceFixture } from "./worker-fence.mjs";
import { until } from "./support.mjs";

process.umask(0o077);
const fixture = new GithubFixture();
const report = { kind: "shaula-github-lifecycle/v1", repository: "5aaee9/shaula", passed: false };
let phase = "setup";
let completed = false;
try {
  await fixture.start();
  report.runtimeTuple = fixture.runtimeTuple;
  report.artifactDigest = fixture.digest;
  const key = `${fixture.prefix}-github`;
  await fixture.createFleet(key);
  phase = "await-browser-dispatch";
  console.log(JSON.stringify({ phase, repository: report.repository, workflow: "shaula-docker-smoke.yml", runner_label: key }));
  const generation = await until("real GitHub job makes the Generation Busy", async () => {
    const rows = (await fixture.api(`/generations?fleet_key=${key}`)).items;
    assert(rows.every(g => !["Quarantined", "CleanupRequired"].includes(g.state)), "GitHub generation unexpectedly quarantined");
    return rows.find(g => g.state === "Busy");
  }, 900_000);
  phase = "busy";
  const ids = await fixture.cli("ps", "-a", "--no-trunc", "-q", "--filter", `label=shaula.fleet=${key}`);
  assert(ids && ids.split("\n").length === 1, "one real Docker resource for one GitHub job");
  fixture.containers.add(ids);
  const volumes = await fixture.containerVolumes(ids);
  const before = workers(fixture, key)[0];
  assert.equal(before.cleanup_only, 0);
  console.log(JSON.stringify({ phase, generation: generation.id, container: ids }));
  // Restart while Busy: this must neither replay JIT/Create nor perform an
  // ordinary unsafe Destroy. The workflow holds Busy for 45 seconds.
  await fixture.restart();
  assert.equal(await fixture.cli("ps", "-a", "--no-trunc", "-q", "--filter", `label=shaula.fleet=${key}`), ids);
  phase = "cleanup";
  await until("GitHub job completed and full cleanup committed", async () => {
    const rows = (await fixture.api(`/generations?fleet_key=${key}`)).items;
    assert.equal(rows.length, 1, "no duplicate generation across restart");
    return rows[0].state === "Destroyed" && (await fixture.api(`/fleets/${key}/status`)).capacity.occupancy === 0;
  }, 300_000);
  assert.equal(await fixture.cli("ps", "-a", "--no-trunc", "-q", "--filter", `label=shaula.fleet=${key}`), "");
  const remainingVolumes = (await fixture.cli("volume", "ls", "-q")).split("\n");
  assert(volumes.every(v => !remainingVolumes.includes(v)), "no bootstrap credential volume remains");
  const after = workers(fixture, key)[0];
  assert(after.worker_epoch > before.worker_epoch && after.phase === "completed");
  report.fleet = key;
  report.generation = generation.id;
  report.workerEpochs = [before.worker_epoch, after.worker_epoch];
  report.jobs = (await fixture.api(`/jobs?fleet_key=${key}`)).items.map(j => ({
    id: j.id, backend: j.backend, observedStatus: j.observed_status, associationStatus: j.association_status,
  }));
  assert(report.jobs.length > 0 && report.jobs.every(j => j.backend === "github"), "real GitHub Jobs retained");
  const current = await fetch(`${fixture.url}/api/v1/fleets/${key}`, {
    headers: { authorization: `Bearer ${fixture.oidc.token}` }, signal: AbortSignal.timeout(5000),
  });
  assert.equal(current.status, 200);
  const version = current.headers.get("shaula-resource-version") || current.headers.get("etag");
  assert(version, "Fleet retirement requires its exact current version");
  await current.body?.cancel();
  await fixture.api(`/fleets/${key}`, "DELETE", undefined, 202, { "if-match": version });
  await until("test Fleet and Scale Set retirement completed", async () => {
    const response = await fetch(`${fixture.url}/api/v1/fleets/${key}`, {
      headers: { authorization: `Bearer ${fixture.oidc.token}` }, signal: AbortSignal.timeout(5000),
    });
    await response.body?.cancel();
    return response.status === 404;
  });
  fixture.containers.delete(ids);
  report.cleanup = "Destroyed; occupancy zero; Docker resource and volumes absent; Fleet retired";
  report.passed = completed = true;
} catch (error) {
  report.failedPhase = phase;
  report.reason = error.message;
  process.exitCode = 1;
} finally {
  // Failure retains the real GitHub evidence and resources for reconciliation;
  // never turn forced fixture teardown into a successful lifecycle receipt.
  try {
    if (completed) await fixture.close();
    else { await fixture.stopDaemon(); await fenceFixture(fixture); await fixture.oidc?.close(); }
  } catch {
    report.passed = false;
    report.teardownFailed = true;
    process.exitCode = 1;
  }
  if (fixture.directory) {
    await writeFile(join(fixture.directory, "github-report.json"), JSON.stringify(report, null, 2), { mode: 0o600 });
    console.log(`Private evidence: ${fixture.directory}`);
  }
  console.log(JSON.stringify(report, null, 2));
}
