#!/usr/bin/env node
// Real official binaries in isolated containers; no Shaula production changes.
import assert from "node:assert/strict";
import { faultEndpoint } from "./fault.mjs";
import { Fixture, runnerImage, serverImage, until } from "./support.mjs";

const fixture = new Fixture();
const results = [];
const cases = [
  { name: "no-task", task: false },
  { name: "task-completes", task: true },
  { name: "task-fails", task: true, fail: true },
  { name: "fetch-unavailable", task: true, fault: "unavailable" },
  { name: "assignment-response-lost", task: true, fault: "lose-response" },
  { name: "wait-recovery-control", task: true, fault: "lose-response", wait: true },
];

async function scenario(spec) {
  const label = `${fixture.prefix}-${spec.name}`;
  if (spec.task) await fixture.queue(spec.name, label, spec.fail);
  else assert.equal((await fixture.jobs(label)).length, 0);
  const registration = await fixture.register(spec.name);
  const endpoint = spec.fault ? await faultEndpoint(fixture.url, spec.fault) : null;
  try {
    const started = Date.now();
    const container = await fixture.runner(
      registration,
      label,
      endpoint?.url ?? fixture.url,
      !!spec.wait,
    );
    const completes = spec.task && (!spec.fault || !!spec.wait);
    let busyPreserved = false;
    if (completes) {
      await until(`${spec.name}: executing`, async () => {
        const running = (await fixture.jobs(label)).some((job) => job.status === "running");
        return running && (await fixture.state(container)).Running;
      });
      busyPreserved = true;
    }
    const state = await until(`${spec.name}: natural exit`, async () => {
      const current = await fixture.state(container);
      return !current.Running && current;
    });
    assert.equal(state.ExitCode, completes ? 0 : 2, `${spec.name}: unexpected exit code`);
    const jobs = await fixture.jobs(label);
    const output = await fixture.command("logs", container);
    assert(!output.includes(registration.token), "runner logs leaked registration token");
    const path = `/admin/actions/runners/${registration.id}`;
    let registrationState;
    let reclamation;
    let workflowStatus = null;
    if (completes) {
      const expected = spec.fail ? "failure" : "success";
      const run = await until(`${spec.name}: ${expected} workflow`, async () => {
        const value = await fixture.api(`/repos/${fixture.user}/${spec.name}/actions/runs`);
        return value.workflow_runs?.find(
          (run) => run.workflow_id === "check.yaml" && run.status === expected,
        );
      });
      workflowStatus = run.status;
      // Successful scope read and exact-ID absence, not list pagination alone.
      await fixture.api("/admin/actions/runners");
      await fixture.api(path, "GET", undefined, 404);
      registrationState = "absent";
      await fixture.removeStopped(container);
      reclamation = "stopped container removed after exact registration absence";
    } else {
      const remote = await fixture.api(path);
      assert.equal(remote.uuid, registration.uuid);
      assert.equal(remote.ephemeral, true);
      registrationState = remote.status;
      if (!spec.task) {
        // This experiment controls the ENTIRE queue and knows no matching job
        // was ever submitted. Production cannot infer this from exit code 2.
        assert.equal(jobs.length, 0);
        await fixture.api(path, "DELETE", undefined, 204);
        await fixture.api(path, "GET", undefined, 404);
        await fixture.removeStopped(container);
        reclamation = "removed using controlled empty-queue knowledge (not production proof)";
      } else {
        assert.equal(jobs.length, 1);
        assert.equal(jobs[0].status, spec.fault === "lose-response" ? "running" : "waiting");
        if (spec.fault === "lose-response") assert(jobs[0].task_id > 0);
        reclamation = "not authorized; registration and container retained until fixture teardown";
      }
    }
    if (endpoint) {
      if (spec.fault === "lose-response") {
        assert.equal(endpoint.evidence.droppedResponses, 1);
        assert.equal(endpoint.evidence.upstreamStatus, 200);
        if (spec.wait) assert(endpoint.evidence.forwardedFetches >= 2);
        else assert.equal(endpoint.evidence.forwardedFetches, 1);
      } else assert.equal(endpoint.evidence.forwardedFetches, 0);
    }
    const result = {
      case: spec.name,
      wait: !!spec.wait,
      exit_code: state.ExitCode,
      elapsed_ms: Date.now() - started,
      busy_preserved_until_natural_exit: busyPreserved,
      task_executed: completes,
      workflow_conclusion: workflowStatus,
      remote_registration_after_exit: registrationState,
      jobs_after_exit: jobs.map(({ status, task_id }) => ({ status, has_task_id: task_id > 0 })),
      fault: endpoint?.evidence ?? null,
      reclamation,
    };
    results.push(result);
    console.error(`PASS ${spec.name}: exit=${state.ExitCode}, registration=${registrationState}`);
  } finally {
    await endpoint?.close();
  }
}

try {
  await fixture.start();
  const images = [];
  for (const image of [serverImage, runnerImage]) {
    const [metadata] = JSON.parse(await fixture.command("image", "inspect", image));
    images.push({ image, id: metadata.Id, repo_digests: metadata.RepoDigests });
  }
  for (const spec of cases) await scenario(spec);
  await fixture.close();
  console.log(
    JSON.stringify(
      {
        experiment: "official non-waiting one-job; not Shaula Pool/Terraform conformance",
        checked_at: new Date().toISOString(),
        engine: fixture.engine,
        images,
        cases: results,
        fixture_resources_removed: true,
        production_bootstrap_changed: false,
      },
      null,
      2,
    ),
  );
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
} finally {
  await fixture.close();
}
