import assert from "node:assert/strict";
import { until } from "./support.mjs";

export async function expiry(fixture, report) {
  await fixture.restart(15);
  const idleKey = `${fixture.prefix}-idle`;
  if (fixture.platform === "docker") fixture.registrationProxy.gate.block = true;
  await fixture.createFleet(idleKey, 1);
  const idle = await fixture.observe(idleKey, "idle");
  await fixture.capZero(idleKey);
  if (fixture.platform === "docker") {
    // An unassigned ephemeral registration is not removed by task completion.
    // This makes controller registration-DELETE retries a separate checkpoint
    // from killing a Busy runner at its hard lifetime.
    await until("idle registration delete fails after resource destruction", () => fixture.registrationProxy.gate.failures > 0);
    assert(!(await fixture.cli("ps", "-a", "--no-trunc", "-q")).split("\n").includes(idle.id));
    assert.equal((await fixture.api(`/fleets/${idleKey}/status`)).capacity.occupancy, 1);
    assert((await fixture.forgejo("/admin/actions/runners")).some(r => r.id === idle.runner));
    const failures = fixture.registrationProxy.gate.failures;
    await fixture.restart();
    await until("registration removal retries across restart", () => fixture.registrationProxy.gate.failures > failures);
    fixture.registrationProxy.gate.block = false;
    report.registrationRetryAcrossRestart = "passed";
  }
  await fixture.reclaimed(idleKey, idle);
  report.idleExpiry = "passed";

  const key = `${fixture.prefix}-busy`;
  await fixture.queue(key, 180);
  await fixture.createFleet(key);
  const busy = await fixture.observe(key, "active");
  // Hard lifetime must converge even with a failed demand read.
  fixture.registrationProxy.gate.failJobs = true;
  if (fixture.platform === "docker") {
    fixture.dockerProxy.gate.block = true;
  }
  await fixture.capZero(key);
  if (fixture.platform === "docker") {
    await until("Docker Destroy failed and remains retryable", () => fixture.dockerProxy.gate.failures > 0);
    assert((await fixture.api(`/fleets/${key}/status`)).capacity.occupancy > 0, "failed Destroy cannot release occupancy");
    // Docker provider 3.0.2 stops before DELETE. A hard-expiry attempt may
    // therefore already have stopped the Runner despite this DELETE outage.
    // Busy-safe ordinary drain is not the contract being exercised here.
    assert((await fixture.cli("ps", "-a", "--no-trunc", "-q")).split("\n").includes(busy.id), "failed removal retains the exact container");
    await fixture.restart();
    fixture.dockerProxy.gate.block = false;
  }
  await fixture.reclaimed(key, busy);
  fixture.registrationProxy.gate.failJobs = false;
  report.busyExpiryDuringStaleDemand = "passed";
  if (fixture.platform === "docker") {
    report.destroyRetryAcrossRestart = "passed";
  }
  await fixture.restart(300);
}
