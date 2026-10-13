// Shared helpers for the spec 0043 §10 G3–G6 suites. Assertions name only
// the inspected surface; no GitLab or daemon body is ever printed.
import assert from "node:assert/strict";
import { join } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { setTimeout as delay } from "node:timers/promises";
import { GitlabFixture } from "./fixture.mjs";
import { gitlabFaultProxy } from "./faults.mjs";
import { until } from "../forgejo-lifecycle/support.mjs";

/** The daemon and Generation runners reach GitLab only through the fault proxy. */
export class ProxiedGitlabFixture extends GitlabFixture {
  get endpoint() { return this.proxyUrl; }
  async startGitlab() {
    await super.startGitlab();
    // Same bridge gateway as GitLab, so Generation containers reach it too.
    const { hostname } = new URL(this.instance);
    this.proxy = await gitlabFaultProxy({ target: this.instance, listen: { host: hostname, port: 0 } });
    this.proxyUrl = `http://${hostname}:${this.proxy.address.port}`;
  }
  async close() {
    try { await super.close(); } finally { await this.proxy?.close(); }
  }
}

/** The persisted demand snapshot (value + proof time), as Forgejo's stale-demand check reads it. */
export function demandRow(fixture, key) {
  const db = new DatabaseSync(join(fixture.directory, "data", "shaula.db"), { readOnly: true });
  try { return db.prepare("SELECT total_assigned_jobs, updated_at FROM fleet_demand WHERE fleet_key = ?").get(key); }
  finally { db.close(); }
}

export const status = (fixture, key) => fixture.api(`/fleets/${key}/status`);
export const generations = async (fixture, key) => (await fixture.api(`/generations?fleet_key=${key}`)).items;

async function current(fixture, key) {
  const response = await fetch(`${fixture.url}/api/v1/fleets/${key}`, {
    headers: { authorization: `Bearer ${fixture.oidc.token}` }, signal: AbortSignal.timeout(5000) });
  assert.equal(response.status, 200, `${key}: Fleet read`);
  const version = response.headers.get("shaula-resource-version") || response.headers.get("etag");
  assert(version, "Fleet writes require the exact current version");
  return { body: await response.json(), version };
}

/** A capacity change is a new Fleet revision: ordinary Retirement for older Generations. */
export async function setCapacity(fixture, key, min, max) {
  const { body, version } = await current(fixture, key);
  body.spec.capacity = { min_runners: min, max_runners: max };
  await fixture.api(`/fleets/${key}`, "PUT", body.spec, 202, { "if-match": version });
}

/** Fleet DELETE without waiting for the tombstone. */
export async function requestRetire(fixture, key) {
  const { version } = await current(fixture, key);
  await fixture.api(`/fleets/${key}`, "DELETE", undefined, 202, { "if-match": version });
}

export async function fleetGone(fixture, key) {
  const response = await fetch(`${fixture.url}/api/v1/fleets/${key}`, {
    headers: { authorization: `Bearer ${fixture.oidc.token}` }, signal: AbortSignal.timeout(5000) });
  await response.body?.cancel();
  return response.status === 410;
}

/** Group registrations with exactly this description (GitLab allows duplicates). */
export async function named(fixture, description) {
  return (await fixture.runners()).filter(r => r.description === description);
}

export async function anchors(fixture, key) {
  return (await fixture.runners()).filter(r => r.description.startsWith(`${key}-anchor-r`));
}

export async function containers(fixture, label) {
  const ids = await fixture.cli("ps", "-a", "--no-trunc", "-q", "--filter", `label=${label}`);
  return ids.split("\n").filter(Boolean);
}

export async function diagnosticCodes(fixture, kind, key) {
  const report = await fixture.api(`/${kind}/${key}/diagnostics`);
  return [...new Set(report.questions.flatMap(q => q.reasons.map(r => r.code)))];
}

/** Holds `check` (which asserts) continuously for `ms`, polling every 2.5 s. */
export async function steady(check, ms) {
  const deadline = Date.now() + ms;
  let passes = 0;
  do { await check(); passes++; await delay(2500); } while (Date.now() < deadline);
  return passes;
}

export async function pipeline(fixture) {
  const created = await fixture.gitlab(`/projects/${fixture.project.id}/pipeline?ref=main`, { method: "POST", expected: 201 });
  return fixture.gitlab(`/projects/${fixture.project.id}/pipelines/${created.id}/jobs`);
}

export const job = (fixture, id) => fixture.gitlab(`/projects/${fixture.project.id}/jobs/${id}`);

export async function jobSucceeded(fixture, id, timeout = 600_000) {
  return until(`job ${id}: success`, async () => {
    const current = await job(fixture, id);
    assert(!["failed", "canceled"].includes(current.status), `job ${id} ${current.status}`);
    return current.status === "success" ? current : false;
  }, timeout);
}

/** One running Generation container of `key` with its exact registration. */
export async function runningGeneration(fixture, key, state, { timeout = 300_000, ignore = [] } = {}) {
  return until(`${key}: Generation ${state} with a running container`, async () => {
    const rows = (await generations(fixture, key)).filter(g => !ignore.includes(g.id));
    assert(rows.every(g => !["Quarantined", "CleanupRequired"].includes(g.state)), `${key}: unexpected quarantine/create failure`);
    const generation = rows.find(g => g.state === state);
    if (!generation) return false;
    const ids = await containers(fixture, `shaula.generation=${generation.id}`);
    if (ids.length !== 1) return false;
    fixture.containers.add(ids[0]);
    const [container] = JSON.parse(await fixture.cli("inspect", ids[0]));
    if (!container.State.Running) return false;
    const registrations = await named(fixture, generation.runner_name);
    assert(registrations.length <= 1, `${key}: duplicate Generation registration`);
    if (!registrations.length) return false;
    return { generation, container: ids[0], runner: registrations[0].id,
      volumes: container.Mounts.filter(m => m.Type === "volume").map(m => m.Name) };
  }, timeout);
}

/** Registration 404, Generation Destroyed, container and its volumes removed. */
export async function reclaimed(fixture, key, observed, timeout = 600_000) {
  await until(`${key}: Generation Destroyed`, async () =>
    (await generations(fixture, key)).find(g => g.id === observed.generation.id)?.state === "Destroyed", timeout);
  await fixture.gitlab(`/runners/${observed.runner}`, { expected: 404 });
  assert(!(await fixture.cli("ps", "-a", "-q", "--no-trunc")).split("\n").includes(observed.container), `${key}: container remains`);
  const volumes = (await fixture.cli("volume", "ls", "-q")).split("\n");
  assert(observed.volumes.every(v => !volumes.includes(v)), `${key}: credential volume remains`);
  fixture.containers.delete(observed.container);
}

/** Retires the Fleet and proves no anchor or Generation registration remains. */
export async function retired(fixture, key) {
  await fixture.retireFleet(key);
  const leftover = (await fixture.runners()).filter(r => r.description.startsWith(`${key}-`));
  assert.equal(leftover.length, 0, `${key}: Fleet retirement leaves no registration`);
  assert.equal((await containers(fixture, `shaula.fleet=${key}`)).length, 0, `${key}: Fleet retirement leaves no container`);
}

/** Out-of-band registration with the operator token; only its ID is kept. */
export async function register(fixture, fields) {
  const { runner_type, group_id, description, maintenance_note, tag_list, run_untagged, locked, paused, access_level } = fields;
  const created = await fixture.gitlab("/user/runners", { method: "POST", expected: 201, token: fixture.token,
    body: { runner_type, group_id, description, maintenance_note, tag_list, run_untagged, locked, paused, access_level } });
  return created.id;
}

/** Domain enums and closed diagnostic codes only, for a failed run's report. */
export async function failureEvidence(fixture) {
  const evidence = [];
  for (const key of fixture.fleets) {
    try {
      const fleet = await status(fixture, key);
      const rows = await generations(fixture, key);
      evidence.push({ key, phase: fleet.phase, lastError: fleet.lastError, capacity: fleet.capacity,
        fleetCodes: await diagnosticCodes(fixture, "fleets", key),
        generations: rows.slice(0, 8).map(g => ({ id: g.id, state: g.state })) });
    } catch { evidence.push({ key, unavailable: true }); }
  }
  return evidence;
}
