import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { workers } from "./worker-pressure.mjs";
import { until } from "./support.mjs";

export async function backendOutage(fixture) {
  const key = `${fixture.prefix}-state-outage`;
  const gate = fixture.dockerProxy.gate;
  const creates = gate.containerCreates;
  const held = gate.heldCreates;
  gate.holdCreateResponses = true;
  await fixture.createFleet(key, 1);
  await until("external Create committed before backend outage", () => gate.heldCreates === held + 1);
  const original = workers(fixture, key)[0];
  const emergencyPath = join(fixture.directory, "data", "runners", original.id, "errored.tfstate");
  await fixture.controlProxy.arm("state_all", "reject_before", 1000);
  gate.holdCreateResponses = false;
  for (const release of gate.heldResponses.splice(0)) release();
  const emergency = await until("actual Terraform retains emergency state", async () => {
    try { return await readFile(emergencyPath); }
    catch (error) { if (error.code === "ENOENT") return false; throw error; }
  });
  const rejected = fixture.controlProxy.counts.matched;
  assert(rejected > 0, "all state methods are really unavailable");
  const containers = await fixture.cli("ps", "-a", "--no-trunc", "-q", "--filter", `label=shaula.fleet=${key}`);
  assert(containers && containers.split("\n").length === 1);
  await fixture.restart();
  await fixture.controlProxy.release();
  await fixture.capZero(key);
  await until("emergency divergence remains quarantined after restart", () => workers(fixture, key)[0].state === "Quarantined");
  await delay(1500);
  assert((await readFile(emergencyPath)).equals(emergency), "no emergency evidence discarded or replaced");
  assert.equal(gate.containerCreates, creates + 1, "restoring HTTP does not authorize re-apply");
  assert.equal((await fixture.api(`/fleets/${key}/status`)).capacity.occupancy, 1);
  assert.equal(await fixture.cli("ps", "-a", "--no-trunc", "-q", "--filter", `label=shaula.fleet=${key}`), containers);
  return { rejectedStateRequests: rejected, emergencyDigest: `sha256:${createHash("sha256").update(emergency).digest("hex")}`,
    creates: 1, createReplays: 0, occupancy: 1, outcome: "quarantined; original workspace and emergency bytes retained" };
}
