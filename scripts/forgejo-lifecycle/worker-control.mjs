// All effects flow through actual serve/job, SQLite and the real Engine.
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { setTimeout as delay } from "node:timers/promises";
import { workers } from "./worker-pressure.mjs";
import { until } from "./support.mjs";

async function version(fixture, key) {
  const response = await fetch(`${fixture.url}/api/v1/fleets/${key}`, {
    headers: { authorization: `Bearer ${fixture.oidc.token}` }, signal: AbortSignal.timeout(5000),
  });
  assert.equal(response.status, 200);
  const value = response.headers.get("shaula-resource-version") || response.headers.get("etag");
  await response.body?.cancel();
  assert(value);
  return value;
}

export async function controlFaults(fixture) {
  const proxy = fixture.controlProxy;
  assert(proxy, "explicit disposable-host control fault proxy required");
  const results = {};
  let key = `${fixture.prefix}-spawn-ack`;
  let creates = fixture.dockerProxy.gate.containerCreates;
  let registrations = fixture.registrationProxy.gate.registrationPosts;
  await proxy.arm("spawned", "drop_after", 2, "create");
  await fixture.createFleet(key, 1);
  const idle = await fixture.observe(key, "idle");
  assert.equal(proxy.counts.committed, 2, "both lost replies followed durable Spawn ACK");
  assert.equal(fixture.dockerProxy.gate.containerCreates, creates + 1);
  assert.equal(fixture.registrationProxy.gate.registrationPosts, registrations + 1);
  await proxy.release();
  await fixture.capZero(key);
  await fixture.queue(key, 40, false, [key], false);
  await fixture.observe(key, "active");
  await fixture.reclaimed(key, idle);
  results.spawnAckLost = { lostReplies: 2, creates: 1, cleanup: "Destroyed" };

  key = `${fixture.prefix}-before-create`;
  creates = fixture.dockerProxy.gate.containerCreates;
  registrations = fixture.registrationProxy.gate.registrationPosts;
  await proxy.arm("handshake", "hold_after");
  await fixture.createFleet(key, 1);
  await until("real exec handshake committed before crash", () => proxy.counts.committed === 1);
  const original = workers(fixture, key)[0];
  assert(original.process_identity);
  await fixture.capZero(key);
  await fixture.restart();
  await proxy.release();
  await until("pre-Create crashed worker is classified", () => workers(fixture, key)[0]?.state === "Destroyed");
  assert.equal(fixture.dockerProxy.gate.containerCreates, creates);
  assert.equal(fixture.registrationProxy.gate.registrationPosts, registrations);
  assert.equal((await fixture.api(`/fleets/${key}/status`)).capacity.occupancy, 0);
  results.crashBeforeCreate = { creates: 0, registrations: 0, cleanup: "NeverStarted; occupancy zero" };

  key = `${fixture.prefix}-delete-race`;
  creates = fixture.dockerProxy.gate.containerCreates;
  await proxy.arm("apply_starting", "hold_before", 1, "create");
  await fixture.createFleet(key, 1);
  await until("Create-start held before its database transaction", () => proxy.counts.matched === 1);
  assert.equal(proxy.counts.committed, 0);
  await fixture.api(`/fleets/${key}`, "DELETE", undefined, 202, { "if-match": await version(fixture, key) });
  await proxy.release();
  await until("deleted Fleet's original attempt settles", () => {
    const rows = workers(fixture, key);
    return rows.length === 1 && ["Destroyed", "Quarantined", "CleanupRequired"].includes(rows[0].state);
  });
  await fixture.restart();
  await delay(1500);
  assert.equal(fixture.dockerProxy.gate.containerCreates, creates, "DELETE commit forbids a late Create spawn");
  results.deleteBeforeCreateStart = { lateCreates: 0, generationState: workers(fixture, key)[0].state };

  key = `${fixture.prefix}-after-intent`;
  creates = fixture.dockerProxy.gate.containerCreates;
  await proxy.arm("apply_starting", "hold_after", 1, "create");
  await fixture.createFleet(key, 1);
  await until("Create-start committed but reply withheld", () => proxy.counts.committed === 1);
  const identity = JSON.parse(workers(fixture, key)[0].process_identity);
  await fixture.restart();
  assert((await readFile(`${identity.containment}/cgroup.events`, "utf8")).includes("populated 0"));
  await proxy.release();
  await fixture.capZero(key);
  await until("ambiguous Create intent remains quarantined", () => workers(fixture, key)[0].state === "Quarantined");
  assert.equal(fixture.dockerProxy.gate.containerCreates, creates, "no Create replay after lost authorization response");
  assert.equal((await fixture.api(`/fleets/${key}/status`)).capacity.occupancy, 1);
  results.crashAfterCreateIntent = { creates: 0, outcome: "quarantined; occupancy one" };
  return results;
}
