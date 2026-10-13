// G4 (spec 0043 §6): ordinary Destroy before the hard timeout never removes
// a Generation that holds a job. The trigger is a capacity change (new
// revision, target 0) that selects an Idle Generation for Retirement; the
// fault proxy holds that Retirement pause while the still-unpaused runner
// claims a job, i.e. the exact race the pause → recheck gate exists for.
// Fleet retirement is then requested while the job runs. Only after the job
// finishes do registration delete, Destroy and the tombstone happen.
import assert from "node:assert/strict";
import { until } from "../forgejo-lifecycle/support.mjs";
import { containers, diagnosticCodes, fleetGone, generations, job, jobSucceeded, pipeline, reclaimed,
  requestRetire, runningGeneration, setCapacity, steady } from "./scenario-support.mjs";

const marker = "SHAULA_GITLAB_BUSY_SAFE";

export async function busySuite(fixture, checks, progress) {
  const { proxy } = fixture;
  const key = `${fixture.prefix}-busy`;
  await fixture.commitCi(`busy:\n  tags: [${key}]\n  script:\n    - echo "${marker} job=$CI_JOB_ID"\n    - sleep 180\n`);
  progress.phase = "idle";
  await fixture.createFleet(key, 1, 1);
  const observed = await runningGeneration(fixture, key, "Idle");

  progress.phase = "retirement-pause";
  proxy.arm({ kind: "pause", mode: "hold_before", runner: observed.runner });
  await setCapacity(fixture, key, 0, 1);
  let claimed;
  try {
    await until(`${key}: Retirement pause issued for the Idle Generation`, () => proxy.gate.held.length === 1, 120_000);
    const heldAt = Date.now();
    const busy = (await pipeline(fixture)).find(j => j.name === "busy");
    // The daemon's request timeout is 30 s; release well before it.
    claimed = await until(`${key}: unpaused runner claims a job while the pause is in flight`, async () => {
      const current = await job(fixture, busy.id);
      return current.runner?.id === observed.runner ? current : false;
    }, 20_000);
    checks.claimBeforePauseMs = Date.now() - heldAt;
  } finally {
    proxy.release();
  }
  const registration = await until(`${key}: pause landed`, async () => {
    const current = await fixture.gitlab(`/runners/${observed.runner}`);
    return current.paused ? current : false;
  }, 30_000);
  assert.equal(registration.description, observed.generation.runner_name);

  progress.phase = "recheck-busy";
  const state = async () => (await generations(fixture, key)).find(g => g.id === observed.generation.id)?.state;
  await until(`${key}: recheck finds the claimed job (Busy), not Retiring`, async () => {
    const current = await state();
    assert(["Idle", "Busy"].includes(current), `${key}: Generation ${current} while its job runs`);
    return current === "Busy";
  }, 120_000);
  await until(`${key}: Retirement recorded cleanup.job_busy`, async () =>
    (await diagnosticCodes(fixture, "generations", observed.generation.id)).includes("cleanup.job_busy"), 60_000);
  const retained = async label => {
    assert.equal(await state(), "Busy", `${label}: Generation retained as Busy`);
    assert.deepEqual(await containers(fixture, `shaula.generation=${observed.generation.id}`), [observed.container], `${label}: container retained`);
    const current = await fixture.gitlab(`/runners/${observed.runner}`);
    assert(current.paused, `${label}: registration stays paused, not deleted`);
    assert.equal((await job(fixture, claimed.id)).status, "running", `${label}: job not interrupted`);
  };
  checks.revisionChangeHeldPasses = await steady(() => retained("revision change"), 35_000);

  progress.phase = "fleet-retire-while-busy";
  await requestRetire(fixture, key);
  checks.fleetRetireHeldPasses = await steady(async () => {
    await retained("Fleet retirement");
    assert(!(await fleetGone(fixture, key)), "no tombstone while a job runs");
  }, 25_000);

  progress.phase = "converge";
  const finished = await jobSucceeded(fixture, claimed.id);
  const trace = await fixture.gitlab(`/projects/${fixture.project.id}/jobs/${claimed.id}/trace`);
  assert(trace.includes(`${marker} job=${claimed.id}`), "job ran to completion on the Generation");
  await reclaimed(fixture, key, observed);
  await until(`${key}: tombstone after the job`, () => fleetGone(fixture, key), 600_000);
  assert.equal((await fixture.runners()).filter(r => r.description.startsWith(`${key}-`)).length, 0, "no anchor or Generation registration remains");
  checks.busySafe = { job: finished.status, durationSeconds: finished.duration,
    sequence: "Idle → Retirement pause raced by a claim → Busy (resources retained) → job success → delete → Destroy → tombstone" };
}
