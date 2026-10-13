// G6 (spec 0043 §3.1, §6): a lost `POST /api/v4/user/runners` response is
// classified by an exact inventory lookup, never by a second POST.
// Generation: ExactlyOne → registration deleted, Generation CleanupRequired;
// None → zero resources and a new Generation; Multiple → Quarantined.
// Anchor: ExactlyOne → adopted; Multiple → all deleted, then rebuilt.
import assert from "node:assert/strict";
import { until } from "../forgejo-lifecycle/support.mjs";
import { anchors, containers, demandRow, generations, job, jobSucceeded, named, reclaimed, register,
  retired, runningGeneration, status, steady } from "./scenario-support.mjs";

const SETTLED = ["CleanupRequired", "Retiring", "DestroyPending", "Destroying", "Destroyed"];

async function anchorLanded(fixture, checks) {
  const { proxy } = fixture;
  const key = `${fixture.prefix}-alost`;
  proxy.arm({ kind: "register", mode: "drop_after", role: "anchor", prefix: `${key}-` });
  await fixture.createFleet(key, 0, 0);
  await until(`${key}: anchor committed, response lost`, () => proxy.gate.committed === 1, 120_000);
  proxy.release();
  const [landed] = await until(`${key}: landed anchor visible`, async () => {
    const found = await named(fixture, `${key}-anchor-r1`);
    return found.length ? found : false;
  });
  await until(`${key}: the unique landing is adopted as proof`, async () =>
    (await status(fixture, key)).phase === "Ready" && demandRow(fixture, key), 120_000);
  await steady(async () => {
    assert.deepEqual((await anchors(fixture, key)).map(a => a.id), [landed.id], `${key}: adopted, not recreated`);
  }, 20_000);
  assert.equal(proxy.posts(`${key}-`).length, 1, `${key}: no second anchor POST`);
  await retired(fixture, key);
  checks.anchorExactlyOne = "adopted the landed anchor; one POST";
}

async function anchorDuplicated(fixture, checks) {
  const { proxy } = fixture;
  const key = `${fixture.prefix}-adup`;
  proxy.arm({ kind: "register", mode: "hold_after", role: "anchor", prefix: `${key}-` });
  await fixture.createFleet(key, 0, 0);
  let ids;
  try {
    await until(`${key}: anchor committed, response held`, () => proxy.gate.held.length === 1, 120_000);
    await register(fixture, proxy.posts(`${key}-`).find(r => r.role === "anchor"));
    ids = (await named(fixture, `${key}-anchor-r1`)).map(r => r.id);
    assert.equal(ids.length, 2, `${key}: an identical second anchor exists before the lookup`);
  } finally {
    proxy.release();
  }
  const rebuilt = await until(`${key}: all candidates deleted, one anchor rebuilt`, async () => {
    const current = await anchors(fixture, key);
    return current.length === 1 && !ids.includes(current[0].id) && current[0].paused ? current[0] : false;
  }, 180_000);
  for (const id of ids) await fixture.gitlab(`/runners/${id}`, { expected: 404 });
  assert.equal(proxy.posts(`${key}-`).length, 2, `${key}: exactly one rebuild POST`);
  await until(`${key}: demand proven on the rebuilt anchor`, async () => (await status(fixture, key)).phase === "Ready", 120_000);
  await retired(fixture, key);
  checks.anchorMultiple = { deleted: ids.length, rebuilt: rebuilt.description };
}

/** Polls the lost Generation's states until it is Destroyed. */
async function lostGeneration(fixture, key, description) {
  const seen = new Set();
  const generation = await until(`${key}: Generation for the lost registration`, async () =>
    (await generations(fixture, key)).find(g => g.runner_name === description));
  await until(`${key}: lost Generation Destroyed`, async () => {
    const current = (await generations(fixture, key)).find(g => g.id === generation.id);
    seen.add(current.state);
    assert(["CreatePending", "Creating", ...SETTLED].includes(current.state), `${key}: lost registration reached ${current.state}`);
    assert.deepEqual(await containers(fixture, `shaula.generation=${generation.id}`), [], `${key}: no resource without a token`);
    return current.state === "Destroyed";
  }, 300_000);
  return { generation, seen: [...seen] };
}

/** The demand is then served by a new Generation's own registration. */
async function retried(fixture, key, lost, pending) {
  const next = await runningGeneration(fixture, key, "Busy", { ignore: [lost.generation.id] });
  assert.notEqual(next.generation.runner_name, lost.generation.runner_name);
  await until(`${key}: job claimed by the retried Generation`, async () => (await job(fixture, pending.id)).runner?.id === next.runner);
  const finished = await jobSucceeded(fixture, pending.id);
  await reclaimed(fixture, key, next);
  await retired(fixture, key);
  return finished.status;
}

async function generationNone(fixture, checks, jobs) {
  const { proxy } = fixture;
  const key = `${fixture.prefix}-none`;
  proxy.arm({ kind: "register", mode: "reset", role: "generation", prefix: `${key}-` });
  await fixture.createFleet(key, 0, 1);
  await until(`${key}: registration request lost before GitLab`, () => proxy.gate.matched === 1, 240_000);
  proxy.release();
  const { description } = proxy.posts(`${key}-`).find(r => r.fault === "reset");
  const lost = await lostGeneration(fixture, key, description);
  assert.equal((await named(fixture, description)).length, 0, `${key}: nothing registered`);
  assert.equal(proxy.posts(`${key}-`).filter(r => r.description === description).length, 1, `${key}: no POST replay`);
  checks.generationNone = { states: lost.seen, retriedJob: await retried(fixture, key, lost, jobs.none) };
}

async function generationExactlyOne(fixture, checks, jobs) {
  const { proxy } = fixture;
  const key = `${fixture.prefix}-one`;
  proxy.arm({ kind: "register", mode: "drop_after", role: "generation", prefix: `${key}-` });
  await fixture.createFleet(key, 0, 1);
  await until(`${key}: registration committed, response lost`, () => proxy.gate.committed === 1, 240_000);
  proxy.release();
  const { description } = proxy.posts(`${key}-`).find(r => r.fault === "drop_after");
  const [landed] = await named(fixture, description);
  assert(landed, `${key}: the registration landed`);
  const lost = await lostGeneration(fixture, key, description);
  assert(lost.seen.includes("CleanupRequired"), `${key}: ExactlyOne enters CleanupRequired`);
  await fixture.gitlab(`/runners/${landed.id}`, { expected: 404 });
  assert.equal(proxy.posts(`${key}-`).filter(r => r.description === description).length, 1, `${key}: no POST replay`);
  checks.generationExactlyOne = { states: lost.seen, registrationDeleted: true, retriedJob: await retried(fixture, key, lost, jobs.one) };
}

async function generationMultiple(fixture, checks) {
  const { proxy } = fixture;
  const key = `${fixture.prefix}-many`;
  proxy.arm({ kind: "register", mode: "hold_after", role: "generation", prefix: `${key}-` });
  await fixture.createFleet(key, 0, 1);
  let description;
  try {
    await until(`${key}: registration committed, response held`, () => proxy.gate.held.length === 1, 240_000);
    const request = proxy.posts(`${key}-`).find(r => r.role === "generation");
    description = request.description;
    await register(fixture, request);
    assert.equal((await named(fixture, description)).length, 2, `${key}: identical duplicate exists before the lookup`);
  } finally {
    proxy.release();
  }
  const generation = await until(`${key}: Multiple → Quarantined`, async () => {
    const found = (await generations(fixture, key)).find(g => g.runner_name === description);
    assert(!found || ["CreatePending", "Creating", "Quarantined"].includes(found.state), `${key}: Multiple reached ${found?.state}`);
    return found?.state === "Quarantined" ? found : false;
  }, 120_000);
  await steady(async () => {
    const rows = await generations(fixture, key);
    assert.deepEqual(rows.map(g => [g.id, g.state]), [[generation.id, "Quarantined"]], `${key}: no new Generation, no cleanup by name`);
    assert.equal((await named(fixture, description)).length, 2, `${key}: neither candidate deleted`);
    assert.equal((await status(fixture, key)).capacity.occupancy, 1, `${key}: quarantine holds occupancy`);
    assert.deepEqual(await containers(fixture, `shaula.fleet=${key}`), [], `${key}: no resource`);
  }, 40_000);
  assert.equal(proxy.posts(`${key}-`).filter(r => r.role === "generation").length, 1, `${key}: no POST replay`);
  // Deliberately not retired: the quarantined Fleet cannot prove removal, and
  // the disposable GitLab server's teardown owns both registrations.
  checks.generationMultiple = "Quarantined; both candidates retained; no replay or Create";
}

export async function uncertainSuite(fixture, checks, progress) {
  const keys = { none: `${fixture.prefix}-none`, one: `${fixture.prefix}-one`, many: `${fixture.prefix}-many` };
  await fixture.commitCi(Object.entries(keys).map(([name, tag]) => `${name}:\n  tags: [${tag}]\n  script: [echo "${name}", sleep 90]\n`).join(""));
  const created = await fixture.gitlab(`/projects/${fixture.project.id}/pipeline?ref=main`, { method: "POST", expected: 201 });
  const pipelineJobs = await fixture.gitlab(`/projects/${fixture.project.id}/pipelines/${created.id}/jobs`);
  const jobs = Object.fromEntries(Object.keys(keys).map(name => [name, pipelineJobs.find(j => j.name === name)]));
  for (const [name, scenario] of [["anchor-exactly-one", anchorLanded], ["anchor-multiple", anchorDuplicated],
    ["generation-none", generationNone], ["generation-exactly-one", generationExactlyOne], ["generation-multiple", generationMultiple]]) {
    progress.phase = name;
    console.log(`Running ${name}`);
    await scenario(fixture, checks, jobs);
  }
}
