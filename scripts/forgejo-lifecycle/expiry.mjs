import assert from "node:assert/strict";
import { until } from "./support.mjs";

export async function expiry(fixture, report) {
  await fixture.restart(15);
  const idleKey = `${fixture.prefix}-idle`;
  await fixture.createFleet(idleKey, 1);
  const idle = await fixture.observe(idleKey, "idle");
  await fixture.capZero(idleKey);
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
    fixture.registrationProxy.gate.block = true;
  }
  await fixture.capZero(key);
  if (fixture.platform === "docker") {
    await until("Docker Destroy failed and remains retryable", () => fixture.dockerProxy.gate.failures > 0);
    assert((await fixture.api(`/fleets/${key}/status`)).capacity.occupancy > 0, "failed Destroy cannot release occupancy");
    assert((await fixture.forgejo("/admin/actions/runners")).some(r => r.id === busy.runner && r.status === "active"), "ordinary drain must preserve a busy registration");
    await fixture.restart();
    fixture.dockerProxy.gate.block = false;
    await until("registration delete is retried after resource destruction", () => fixture.registrationProxy.gate.failures > 0);
    assert(!(await fixture.cli("ps", "-a", "--no-trunc", "-q")).split("\n").includes(busy.id), "hard expiry must destroy the resource before registration removal");
    assert((await fixture.api(`/fleets/${key}/status`)).capacity.occupancy > 0, "registration failure retains occupancy");
    assert((await fixture.forgejo("/admin/actions/runners")).some(r => r.id === busy.runner), "registration retry checkpoint exercised");
    await fixture.restart();
    fixture.registrationProxy.gate.block = false;
  }
  await fixture.reclaimed(key, busy);
  fixture.registrationProxy.gate.failJobs = false;
  report.busyExpiryDuringStaleDemand = "passed";
  if (fixture.platform === "docker") {
    report.destroyRetryAcrossRestart = "passed";
    report.registrationRetryAcrossRestart = "passed";
  }
  await fixture.restart(300);
}
