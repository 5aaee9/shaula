#!/usr/bin/env node
// Actual shaula serve + real GitLab CE + real Terraform/Docker: the spec 0043
// §10 G3–G6 suites, one per run (SHAULA_GITLAB_SUITE). The G1/G2/G7/G8
// baseline stays in run.mjs. Fault suites put a transport proxy between the
// daemon (and its runners) and GitLab; nothing in the binary is simulated.
import assert from "node:assert/strict";
import { writeFile } from "node:fs/promises";
import { join } from "node:path";
import { GitlabFixture, serverImage } from "./fixture.mjs";
import { ProxiedGitlabFixture, failureEvidence } from "./scenario-support.mjs";
import { demandSuite } from "./scenarios-demand.mjs";
import { busySuite } from "./scenarios-busy.mjs";
import { tokenSuite } from "./scenarios-token.mjs";
import { uncertainSuite } from "./scenarios-uncertain.mjs";

process.umask(0o077);
const suites = {
  demand: { run: demandSuite, proxied: true, covers: "G3" },
  busy: { run: busySuite, proxied: true, covers: "G4" },
  token: { run: tokenSuite, proxied: false, covers: "G5" },
  uncertain: { run: uncertainSuite, proxied: true, covers: "G6" },
};
const suite = process.env.SHAULA_GITLAB_SUITE;
assert(Object.hasOwn(suites, suite), "SHAULA_GITLAB_SUITE must be demand, busy, token or uncertain");
const selected = suites[suite];
const fixture = selected.proxied ? new ProxiedGitlabFixture() : new GitlabFixture();
const report = { kind: "shaula-gitlab-lifecycle/v1", suite, covers: selected.covers, platform: "docker", serverImage, checks: {}, passed: false };
const progress = { phase: "setup" };
let completed = false;
try {
  await fixture.start();
  Object.assign(report, { runtimeTuple: fixture.runtimeTuple, engineVersion: fixture.engine.Version,
    artifactDigest: fixture.digest, runnerImage: fixture.runnerImage, gitlabVersion: (await fixture.gitlab("/version")).version });
  await selected.run(fixture, report.checks, progress);
  report.passed = completed = true;
} catch (error) {
  report.failedPhase = progress.phase;
  // No bodies, secret-bearing subprocess output or raw server logs in reports.
  report.reason = error.message;
  if (fixture.url) report.failureEvidence = await failureEvidence(fixture);
  process.exitCode = 1;
} finally {
  // Request classes only; the proxy keeps no headers or response bodies.
  if (fixture.proxy) report.proxyRequests = { ...fixture.proxy.gate.counts };
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
