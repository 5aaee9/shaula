// G7 (spec 0043 §6.1): the Jobs API shows the claim's Verified association,
// and that association survives deletion of the registration, after which
// GitLab itself reports `job.runner = null`. Reports carry IDs and states only.
import assert from "node:assert/strict";
import { until } from "../forgejo-lifecycle/support.mjs";

async function detail(fixture, key, job) {
  const page = await fixture.api(`/jobs?fleet_key=${encodeURIComponent(key)}&limit=200`);
  const items = page.items.filter(item => item.backend === "gitlab" && item.gitlab?.job_id === String(job.id));
  assert(items.length <= 1, "one Jobs record per project_id/job_id");
  return items[0] ? fixture.api(`/jobs/${encodeURIComponent(items[0].id)}`) : false;
}

function verified(record, fixture, generation, pipeline, job) {
  assert.equal(record.association_status, "verified");
  assert.equal(record.protocol_job_id, `${fixture.project.id}:${job.id}`);
  assert.equal(record.scale_set_id, undefined, "GitLab records never carry a scale_set_id");
  assert.equal(record.github_run_url, null);
  assert.equal(record.gitlab.project_id, String(fixture.project.id));
  assert.equal(record.gitlab.project_path, fixture.project.path_with_namespace);
  assert.equal(record.gitlab.pipeline_id, String(pipeline.id));
  assert.equal(record.gitlab.job_name, job.name);
  assert(record.gitlab.web_url.endsWith(`/${fixture.project.path_with_namespace}/-/jobs/${job.id}`), "web_url names the exact job");
  assert.deepEqual(record.generations.map(g => [g.id, g.association_status]), [[generation, "verified"]]);
}

/** Right after the claim: the record exists, running or queued, with its Generation. */
export async function jobsAfterClaim(fixture, key, generation, pipeline, job) {
  const record = await until(`${key}: Jobs API shows the claimed GitLab job`, () => detail(fixture, key, job), 120_000);
  verified(record, fixture, generation, pipeline, job);
  assert(["running", "queued"].includes(record.observed_status), `claimed job is ${record.observed_status}`);
  return { status: record.observed_status, freshness: record.freshness, association: record.association_status,
    generations: record.generations.map(g => g.id), runnerJobsEvents: record.gitlab_observations.filter(e => e.source === "runner_jobs").length };
}

/** After reclaim: the registration is gone, the job's own success is read, the association stays. */
export async function jobsAfterReclaim(fixture, key, generation, pipeline, job) {
  const upstream = await fixture.gitlab(`/projects/${fixture.project.id}/jobs/${job.id}`);
  const record = await until(`${key}: Jobs API records the job's own terminal status`, async () => {
    const found = await detail(fixture, key, job);
    return found?.reported_result ? found : false;
  }, 180_000);
  verified(record, fixture, generation, pipeline, job);
  assert.equal(record.observed_status, "completed");
  assert.equal(record.reported_result, "success");
  assert.equal(record.freshness, "confirmed");
  assert.equal(record.generations[0].state, "Destroyed");
  const owner = await fixture.api(`/generations/${generation}`);
  assert.equal(owner.association_status, "verified");
  assert(owner.jobs.some(item => item.id === record.id), "the Generation lists its Verified job");
  return { status: record.observed_status, result: record.reported_result, freshness: record.freshness,
    generations: record.generations.map(g => [g.id, g.state, g.association_status]),
    upstreamRunnerAfterDelete: upstream.runner?.id ?? null,
    jobReadEvents: record.gitlab_observations.filter(e => e.source === "job_read").length };
}
