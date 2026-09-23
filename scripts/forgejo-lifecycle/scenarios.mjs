// Real-server checks; transports may fail management reads/writes, never acquisition.
import assert from "node:assert/strict";
import { DatabaseSync } from "node:sqlite";
import { join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { until } from "./support.mjs";

async function capacity(fixture, key, min, max) {
  const response = await fetch(`${fixture.url}/api/v1/fleets/${key}`, { headers: { authorization: `Bearer ${fixture.oidc.token}` } });
  assert.equal(response.status, 200);
  const body = await response.json();
  body.spec.capacity = { min_runners: min, max_runners: max };
  await fixture.api(`/fleets/${key}`, "PUT", body.spec, 202, { "if-match": response.headers.get("shaula-resource-version") || response.headers.get("etag") });
}
function demand(fixture, key) {
  const db = new DatabaseSync(join(fixture.directory, "data", "shaula.db"), { readOnly: true });
  try { return db.prepare("SELECT total_assigned_jobs,updated_at FROM fleet_demand WHERE fleet_key=?").get(key); }
  finally { db.close(); }
}

export async function labels(fixture) {
  const key = `${fixture.prefix}-labels`;
  const other = `${fixture.prefix}-mismatch`;
  await fixture.queue(other, 1, false, [key, "unavailable-label"]);
  await fixture.createFleet(key);
  await until("nonmatching labels are observed without demand", () => demand(fixture, key));
  await delay(1500);
  assert.equal((await fixture.api(`/generations?fleet_key=${key}`)).items.length, 0, "unmatched job must not create a runner");
  await fixture.queue(key, 8);
  const observed = await fixture.observe(key, "active");
  await fixture.reclaimed(key, observed);
  const unmatched = await fixture.forgejo(`/admin/actions/runners/jobs?labels=${key},unavailable-label`);
  assert(unmatched.some(job => job.status === "waiting"), "unmatched job must remain queued");
}

export async function staleDemand(fixture) {
  const key = `${fixture.prefix}-stale`;
  const idleKey = `${fixture.prefix}-retained-idle`;
  await fixture.createFleet(idleKey, 1);
  const idle = await fixture.observe(idleKey, "idle");
  await fixture.queue(key, 8);
  await fixture.createFleet(key, 0, 0);
  await until("positive waiting demand retained at zero capacity", () => demand(fixture, key)?.total_assigned_jobs === 1);
  fixture.registrationProxy.gate.failJobs = true;
  const failures = fixture.registrationProxy.gate.jobFailures;
  await until("management jobs poll fails", () => fixture.registrationProxy.gate.jobFailures > failures);
  const before = demand(fixture, key);
  await capacity(fixture, key, 0, 1);
  await fixture.capZero(idleKey);
  await fixture.restart();
  const failedAfterRestart = fixture.registrationProxy.gate.jobFailures;
  await until("failed polls repeat across restart", () => fixture.registrationProxy.gate.jobFailures >= failedAfterRestart + 4);
  assert.deepEqual(demand(fixture, key), before, "poll failure must preserve demand value and time");
  assert.equal((await fixture.api(`/generations?fleet_key=${key}`)).items.length, 0, "stale demand must not authorize Create");
  assert.equal((await fixture.api(`/fleets/${idleKey}/status`)).capacity.occupancy, 1, "stale demand must not authorize ordinary drain");
  assert((await fixture.forgejo("/admin/actions/runners")).some(r => r.id === idle.runner), "idle registration retained");
  fixture.registrationProxy.gate.failJobs = false;
  const active = await fixture.observe(key, "active");
  await fixture.reclaimed(key, active);
  // Consume the waiting runner normally: no idle-fence claim is made here.
  await fixture.queue(idleKey, 8);
  await fixture.observe(idleKey, "active");
  await fixture.reclaimed(idleKey, idle);
}

export async function lostRegistration(fixture) {
  const key = `${fixture.prefix}-lost`;
  const gate = fixture.registrationProxy.gate;
  const before = gate.registrationPosts;
  gate.dropRegistrationResponses = true;
  await fixture.createFleet(key, 1);
  await until("lost registration is quarantined", async () => (await fixture.api(`/generations?fleet_key=${key}`)).items.some(g => g.state === "Quarantined"));
  assert.equal(gate.registrationPosts, before + 1, "registration POST must not retry");
  assert.equal(gate.droppedRegistrations, 1, "real server must commit before response loss");
  await fixture.restart();
  await delay(2500);
  assert.equal(gate.registrationPosts, before + 1, "restart must not replay an uncertain POST");
  assert.equal((await fixture.api(`/fleets/${key}/status`)).capacity.occupancy, 1, "quarantine must retain occupancy");
  const runners = (await fixture.forgejo("/admin/actions/runners")).filter(r => r.name.startsWith(`${key}-`));
  assert.equal(runners.length, 1, "one undeclared orphan retained, not claimed by name alone");
  if (fixture.platform === "kubernetes") {
    assert.equal(JSON.parse(await fixture.kube("get", "pods,secrets", "-l", `shaula.io/fleet=${key}`, "-o", "json")).items.length, 0, "no Kubernetes resource from a lost credential");
  } else {
    assert.equal(await fixture.cli("ps", "-a", "--no-trunc", "-q", "--filter", `label=shaula.fleet=${key}`), "", "no Terraform resource from a lost credential");
  }
  gate.dropRegistrationResponses = false;
  // This orphan is deliberately NOT counted as Shaula reclaim; fixture server teardown owns it.
}
