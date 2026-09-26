import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { setTimeout as delay } from "node:timers/promises";
import { workers } from "./worker-pressure.mjs";
import { until } from "./support.mjs";

export async function interruptedCreate(fixture) {
  const key = `${fixture.prefix}-interrupted-create`;
  const gate = fixture.dockerProxy.gate;
  const before = gate.containerCreates;
  const registrations = fixture.registrationProxy.gate.registrationPosts;
  gate.holdCreateResponses = true;
  await fixture.createFleet(key, 1);
  await until("Engine accepts Create before losing its response", () => gate.heldCreates === 1);
  const row = workers(fixture, key)[0];
  assert(row?.process_identity, "real worker identity must already be durable");
  const identity = JSON.parse(row.process_identity);
  const containers = await fixture.cli("ps", "-a", "--no-trunc", "-q", "--filter", `label=shaula.fleet=${key}`);
  assert(containers, "provider really committed the resource");
  assert.equal(containers.split("\n").length, 1);
  await fixture.stopDaemon();
  // Parent-pipe closure must terminate the actual worker/provider subtree.
  await until("daemon SIGKILL fences descendants", async () =>
    (await readFile(`${identity.containment}/cgroup.events`, "utf8")).includes("populated 0"));
  gate.holdCreateResponses = false;
  for (const release of gate.heldResponses.splice(0)) release();
  await fixture.startDaemon();
  await until("uncertain accepted Create is quarantined", async () =>
    (await fixture.api(`/generations?fleet_key=${key}`)).items.some(g => g.state === "Quarantined"));
  await fixture.capZero(key);
  await fixture.restart();
  await delay(2500);
  assert.equal(gate.containerCreates, before + 1, "neither restart may repeat Create");
  assert.equal(fixture.registrationProxy.gate.registrationPosts, registrations + 1, "no CI registration replay");
  assert.equal((await fixture.api(`/fleets/${key}/status`)).capacity.occupancy, 1);
  assert.equal(await fixture.cli("ps", "-a", "--no-trunc", "-q", "--filter", `label=shaula.fleet=${key}`), containers,
    "partial/empty state cannot hide the accepted external effect");
  return { daemonSignal: "SIGKILL", acceptedCreates: 1, createReplays: 0,
    descendantFence: "populated 0", restarts: 2, occupancy: 1,
    outcome: "quarantined; external resource retained for operator reconciliation" };
}
