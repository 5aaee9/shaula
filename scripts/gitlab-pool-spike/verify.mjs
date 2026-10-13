// Disposable GitLab CE + gitlab-runner protocol experiment for the proposed GitLab
// Pool backend. Verifies upstream facts only, not Shaula integration.
import assert from "node:assert/strict";
import { Fixture, runnerImage, serverImage, until } from "./support.mjs";

const tag = "shaula-spike";
const ci = `match:
  tags: [${tag}]
  script:
    - echo "TASK_EXECUTED job=$CI_JOB_ID runner=$CI_RUNNER_ID"
    - sleep 20
mismatch:
  tags: [${tag}, gpu]
  script: [echo must-stay-pending]
untagged:
  script: [echo must-stay-pending]
`;

const fixture = new Fixture();
const report = { generatedAt: new Date().toISOString(), cases: {} };

const gid = (id) => `gid://gitlab/Ci::Runner/${id}`;
const tokenOf = (registration) => registration.token;

async function setup() {
  const group = (await fixture.api("/groups", {
    method: "POST",
    body: { name: "spike", path: "spike", visibility: "private" },
    expected: 201,
  })).body;
  const project = (await fixture.api("/projects", {
    method: "POST",
    body: { name: "alpha", namespace_id: group.id, visibility: "private", initialize_with_readme: true },
    expected: 201,
  })).body;
  const expires = new Date(Date.now() + 2 * 86_400_000).toISOString().slice(0, 10);
  // The only credential used for Shaula-side calls: Owner group access token with
  // create_runner + manage_runner + read_api. The admin token only prepares fixtures.
  const operator = (await fixture.api(`/groups/${group.id}/access_tokens`, {
    method: "POST",
    body: { name: "shaula", scopes: ["create_runner", "manage_runner", "read_api"], access_level: 50, expires_at: expires },
    expected: 201,
  })).body.token;
  await fixture.api(`/projects/${project.id}/repository/commits`, {
    method: "POST",
    body: {
      branch: "main",
      commit_message: "ci",
      actions: [{ action: "create", file_path: ".gitlab-ci.yml", content: ci }],
    },
    expected: 201,
  });
  return { group, project, operator };
}

async function register(env, description, extra = {}) {
  const { body } = await fixture.api("/user/runners", {
    method: "POST",
    token: env.operator,
    body: {
      runner_type: "group_type",
      group_id: env.group.id,
      description,
      maintenance_note: `shaula:generation=${description}`,
      tag_list: [tag],
      run_untagged: false,
      ...extra,
    },
    expected: 201,
  });
  assert.deepEqual(Object.keys(body).sort(), ["id", "token", "token_expires_at"]);
  assert.match(body.token, /^glrt-/);
  return body;
}

async function compatible(env, runnerId, token = env.operator) {
  const data = await fixture.graphql(
    `{ jobs(statuses: [PENDING], compatibleRunnerId: "${gid(runnerId)}") { count nodes { id name } }
       runner(id: "${gid(runnerId)}") { id userPermissions { updateRunner } } }`,
    token,
  );
  return {
    count: data.jobs.count,
    names: data.jobs.nodes.map((job) => job.name).sort(),
    runnerVisible: data.runner !== null,
    runnerManageable: data.runner?.userPermissions.updateRunner === true,
  };
}

async function pipeline(env) {
  await fixture.api(`/projects/${env.project.id}/pipeline`, { method: "POST", body: { ref: "main" }, expected: 201 });
}

async function job(env, id) {
  return (await fixture.api(`/projects/${env.project.id}/jobs/${id}`)).body;
}

async function runningOn(env, runnerId) {
  return (await fixture.api(`/runners/${runnerId}/jobs?status=running`, { token: env.operator })).body;
}

async function remove(env, runnerId) {
  const deleted = await fixture.api(`/runners/${runnerId}`, { method: "DELETE", token: env.operator, expected: 204 });
  const after = await fixture.api(`/runners/${runnerId}`, { token: env.operator, expected: null });
  return { deleteStatus: deleted.status, afterStatus: after.status };
}

async function leakScan(id, registration) {
  const container = await fixture.inspect(id);
  const logs = await fixture.logs(id);
  const token = tokenOf(registration);
  const short = /with token (\S+)/.exec(logs)?.[1];
  return {
    argvContainsToken: JSON.stringify(container.Config.Cmd ?? []).includes(token),
    envContainsToken: JSON.stringify(container.Config.Env ?? []).includes(token),
    logsContainFullToken: logs.includes(token),
    logsShortTokenIsTokenSubstring: Boolean(short && token.includes(short)),
  };
}

async function demandCase(env) {
  const anchor = await register(env, `${fixture.prefix}-anchor`, { paused: true });
  const before = await until("first pipeline pending", async () => {
    const current = await compatible(env, anchor.id);
    return current.count === 1 && current;
  }, 120_000, 3_000);
  const all = (await fixture.api(`/projects/${env.project.id}/jobs?scope[]=pending`, { token: env.operator })).body;
  const unfiltered = await fixture.graphql("{ jobs(statuses: [PENDING]) { count } }", env.operator);
  const instance = (await fixture.api("/user/runners", {
    method: "POST",
    body: { runner_type: "instance_type", description: `${fixture.prefix}-foreign`, tag_list: [tag], run_untagged: false, paused: true },
    expected: 201,
  })).body;
  const foreign = await compatible(env, instance.id);
  const foreignAsAdmin = await compatible(env, instance.id, fixture.adminToken);
  assert.equal(before.count, 1);
  assert.deepEqual(before.names, ["match"]);
  assert.equal(unfiltered.jobs.count, 0);
  // A foreign runner may still be visible; only manage permission proves the filter is authorized.
  assert.equal(before.runnerManageable, true);
  assert.equal(foreign.count, 0);
  assert.equal(foreign.runnerManageable, false);
  assert.equal(foreignAsAdmin.count, 1);
  return {
    env: { anchor: anchor.id, foreign: instance.id },
    result: {
      projectPendingJobs: all.map((entry) => ({ name: entry.name, tags: entry.tag_list.sort() })),
      pausedAnchorCompatible: before,
      operatorWithoutRunnerFilterCount: unfiltered.jobs.count,
      operatorOnForeignRunner: foreign,
      adminOnForeignRunnerCount: foreignAsAdmin.count,
    },
  };
}

async function closedLoopCase(env) {
  const registration = await register(env, `${fixture.prefix}-g1`);
  const container = await fixture.runner(registration, "g1", 120);
  const running = await until("g1 picks a job", async () => (await runningOn(env, registration.id))[0]);
  const runner = (await fixture.api(`/runners/${registration.id}`, { token: env.operator })).body;
  const busy = { status: runner.status, jobExecutionStatus: runner.job_execution_status, jobRunnerId: (await job(env, running.id)).runner?.id };
  const state = await fixture.exited(container);
  const finished = await job(env, running.id);
  const retained = (await fixture.api(`/runners/${registration.id}`, { token: env.operator })).body;
  const result = {
    busy,
    exitCode: state.ExitCode,
    jobStatus: finished.status,
    jobRunnerIdAfterExit: finished.runner?.id,
    registrationAfterExit: { exists: true, status: retained.status, jobExecutionStatus: retained.job_execution_status },
    leakScan: await leakScan(container, registration),
    removal: await remove(env, registration.id),
  };
  assert.equal(busy.jobRunnerId, registration.id);
  assert.equal(busy.jobExecutionStatus, "active");
  assert.equal(result.exitCode, 0);
  assert.equal(result.jobStatus, "success");
  assert.equal(result.leakScan.argvContainsToken || result.leakScan.envContainsToken || result.leakScan.logsContainFullToken, false);
  return result;
}

async function noJobCase(env) {
  const registration = await register(env, `${fixture.prefix}-g2`);
  assert.equal((await compatible(env, registration.id)).count, 0);
  const started = Date.now();
  const container = await fixture.runner(registration, "g2", 10);
  const state = await fixture.exited(container);
  const jobs = (await fixture.api(`/runners/${registration.id}/jobs`, { token: env.operator })).body;
  const result = {
    exitCode: state.ExitCode,
    elapsedSeconds: Math.round((Date.now() - started) / 1000),
    jobsProcessed: jobs.length,
    removal: await remove(env, registration.id),
  };
  assert.equal(result.exitCode, 0);
  assert.equal(result.jobsProcessed, 0);
  return result;
}

async function pause(env, runnerId, paused) {
  const { body } = await fixture.api(`/runners/${runnerId}`, { method: "PUT", token: env.operator, body: { paused } });
  assert.equal(body.paused, paused);
}

async function pauseCase(env, anchor) {
  await pipeline(env);
  await until("pending match job", async () => (await compatible(env, anchor)).count === 1);
  const idle = await register(env, `${fixture.prefix}-g5`);
  await pause(env, idle.id, true);
  const idleState = await fixture.exited(await fixture.runner(idle, "g5", 15));
  const idleJobs = (await fixture.api(`/runners/${idle.id}/jobs`, { token: env.operator })).body;
  const pausedBeforeStart = {
    exitCode: idleState.ExitCode,
    jobsProcessed: idleJobs.length,
    compatibleStillPending: (await compatible(env, anchor)).count,
    removal: await remove(env, idle.id),
  };
  const busy = await register(env, `${fixture.prefix}-g6`);
  const container = await fixture.runner(busy, "g6", 120);
  const running = await until("g6 picks a job", async () => (await runningOn(env, busy.id))[0]);
  await pause(env, busy.id, true);
  const runner = (await fixture.api(`/runners/${busy.id}`, { token: env.operator })).body;
  const whilePaused = {
    runningJobs: (await runningOn(env, busy.id)).map((entry) => entry.id === running.id),
    jobRunnerId: (await job(env, running.id)).runner?.id === busy.id,
    jobExecutionStatus: runner.job_execution_status,
  };
  const state = await fixture.exited(container);
  const finished = await job(env, running.id);
  const result = {
    pausedBeforeStart,
    pausedWhileRunning: {
      ...whilePaused,
      exitCode: state.ExitCode,
      finalJobStatus: finished.status,
      jobRunnerRetained: finished.runner?.id === busy.id,
      removal: await remove(env, busy.id),
    },
  };
  assert.equal(pausedBeforeStart.jobsProcessed, 0);
  assert.equal(pausedBeforeStart.compatibleStillPending, 1);
  assert.deepEqual(whilePaused.runningJobs, [true]);
  assert.equal(result.pausedWhileRunning.finalJobStatus, "success");
  return result;
}

async function deleteWhileRunningCase(env) {
  await pipeline(env);
  const registration = await register(env, `${fixture.prefix}-g3`);
  const container = await fixture.runner(registration, "g3", 120);
  const running = await until("g3 picks a job", async () => (await runningOn(env, registration.id))[0]);
  const removal = await remove(env, registration.id);
  const afterDelete = await job(env, running.id);
  const state = await fixture.exited(container);
  const finished = await until("job terminal", async () => {
    const current = await job(env, running.id);
    return ["success", "failed", "canceled"].includes(current.status) && current;
  });
  const result = {
    removal,
    jobStatusRightAfterDelete: afterDelete.status,
    jobRunnerAfterDelete: afterDelete.runner,
    exitCode: state.ExitCode,
    finalJobStatus: finished.status,
  };
  assert.equal(result.finalJobStatus, "success");
  assert.equal(result.jobRunnerAfterDelete, null);
  return result;
}

async function tokenReuseCase(env) {
  await pipeline(env);
  await pipeline(env);
  const registration = await register(env, `${fixture.prefix}-g4`);
  const first = await fixture.runner(registration, "g4a", 60);
  const second = await fixture.runner(registration, "g4b", 60);
  const concurrent = await until("two concurrent jobs on one registration", async () => {
    const jobs = await runningOn(env, registration.id);
    return jobs.length === 2 && jobs.map((entry) => entry.id);
  });
  await fixture.exited(first);
  await fixture.exited(second);
  const result = { concurrentJobsOnOneRegistration: concurrent.length, removal: await remove(env, registration.id) };
  assert.equal(result.concurrentJobsOnOneRegistration, 2);
  return result;
}

async function identityCase(env) {
  const description = `${fixture.prefix}-dup`;
  const a = await register(env, description);
  const b = await register(env, description);
  const listed = (await fixture.api(`/groups/${env.group.id}/runners?type=group_type&per_page=100`, { token: env.operator })).body;
  const matches = listed.filter((runner) => runner.description === description);
  const note = (await fixture.api(`/runners/${a.id}`, { token: env.operator })).body.maintenance_note;
  const result = {
    duplicateDescriptionAccepted: a.id !== b.id,
    exactDescriptionMatches: matches.length,
    maintenanceNoteRoundTrip: note === `shaula:generation=${description}`,
  };
  await remove(env, a.id);
  await remove(env, b.id);
  assert.equal(result.exactDescriptionMatches, 2);
  assert.equal(result.maintenanceNoteRoundTrip, true);
  return result;
}

try {
  await fixture.start();
  const versions = (await fixture.api("/version")).body;
  report.versions = { gitlab: versions.version, revision: versions.revision, runnerImage, serverImage };
  report.imageIds = {
    server: (await fixture.inspect(fixture.server)).Image,
    runner: JSON.parse(await fixture.command("image", "inspect", runnerImage))[0].Id,
  };
  const env = await setup();
  const demand = await demandCase(env);
  report.cases.demand = demand.result;
  report.cases.closedLoop = await closedLoopCase(env);
  report.cases.noJob = await noJobCase(env);
  report.cases.pause = await pauseCase(env, demand.env.anchor);
  report.cases.deleteWhileRunning = await deleteWhileRunningCase(env);
  report.cases.tokenReuse = await tokenReuseCase(env);
  report.cases.identity = await identityCase(env);
  const mismatch = (await fixture.api(`/projects/${env.project.id}/jobs?scope[]=pending&per_page=100`)).body;
  report.cases.mismatchStillPending = mismatch.filter((entry) => entry.name !== "match").length;
  assert(report.cases.mismatchStillPending >= 2);
} finally {
  await fixture.close();
}
// Only emitted after every assertion and teardown succeeded.
process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
