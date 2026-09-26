#!/usr/bin/env node
import assert from "node:assert/strict";
import { writeFile, readFile } from "node:fs/promises";
import { join } from "node:path";
import { createHash } from "node:crypto";
import { DiagnosticsFixture } from "./diagnostics-fixture.mjs";
import { Evidence } from "./diagnostics-evidence.mjs";
import { noCreate, waitingOnline, destroyFailure, rolloutLag } from "./diagnostics-scenarios.mjs";
import { command, serverImage, runnerImage } from "./support.mjs";

process.umask(0o077);
assert.equal(process.env.SHAULA_DX30_DISPOSABLE_VM, "1", "explicit disposable VM opt-in required");
const f = new DiagnosticsFixture();
const evidence = new Evidence(f);
const report = { kind: "shaula-diagnostics-dx30/v1", passed: false, checks: {}, platform: "docker", serverImage, runnerImage };
let phase = "setup";
try {
  await f.start();
  report.engineVersion = f.engine.Version;
  report.forgejoVersion = (await (await fetch(`${f.direct}/api/v1/version`)).json()).version;
  report.terraformVersion = JSON.parse(await command(f.terraform, ["version", "-json"])).terraform_version;
  report.providerVersion = "3.0.2";
  report.binarySha256 = createHash("sha256").update(await readFile(f.binary)).digest("hex");
  report.artifactDigest = f.digest;
  await evidence.start();
  for (const [name, scenario] of [["no-create", noCreate], ["waiting-online", waitingOnline], ["destroy-failure", destroyFailure], ["rollout-lag", rolloutLag]]) {
    phase = name;
    console.log(`DX-30: ${phase}`);
    await scenario(f, evidence);
    report.checks[name] = "passed";
  }
  report.faults = { imageRead: f.dockerProxy.gate.imageFailures, declare: f.registrationProxy.gate.declareFailures,
    resourceDelete: f.dockerProxy.gate.failures, registrationDelete: f.registrationProxy.gate.failures };
  report.receipts = evidence.receipts;
  report.passed = true;
} catch (error) {
  report.failedPhase = phase;
  report.reason = error.message;
  process.exitCode = 1;
} finally {
  await evidence.close();
  try { await f.close(); }
  catch { report.passed = false; report.teardownFailed = true; process.exitCode = 1; }
  if (f.directory) {
    // A failed report cannot be mistaken for acceptance, and raw fixture state
    // stays separate from the explicitly allowlisted public evidence directory.
    await writeFile(join(f.directory, "dx30-report.json"), JSON.stringify(report, null, 2), { mode: 0o600 });
    console.log(`Private fixture directory: ${f.directory}`);
  }
  console.log(JSON.stringify(report, null, 2));
}
