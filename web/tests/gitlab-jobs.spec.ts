import { expect, test, type Page } from "@playwright/test";
import { mockApi } from "./fixtures";

const job = {
  id: "gitlab-job",
  backend: "gitlab",
  fleet_key: "gitlab-build",
  fleet_incarnation: "inc-1",
  protocol_job_id: "18:9007199254740993",
  owner_name: null,
  repository_name: null,
  job_display_name: "match",
  job_workflow_ref: null,
  workflow_run_id: null,
  workflow_run_attempt: null,
  observed_status: "running",
  reported_result: null,
  github_conclusion: null,
  github_run_url: null,
  association_status: "verified",
  freshness: "fresh",
  created_at: 1800000000000,
  updated_at: 1800000001000,
  gitlab: {
    target: {
      instance_url: "https://gitlab.example.test",
      scope: { kind: "group", path: "platform/ci" },
    },
    project_id: "18",
    project_path: "platform/ci/app",
    job_id: "9007199254740993",
    pipeline_id: "77",
    job_name: "match",
    web_url: "https://gitlab.example.test/platform/ci/app/-/jobs/9007199254740993",
    last_reported_status: "running",
    last_observed_at: 1800000001000,
    listed: true,
  },
};
// The Generation is Destroyed and its registration deleted: GitLab now reports
// `runner: null`, yet the recorded Verified association stays visible (G7).
const generation = {
  id: "gitlab-gen-1",
  fleet_key: "gitlab-build",
  fleet_incarnation: null,
  runner_name: "gitlab-build-1",
  generation_name: "gitlab-gen-1",
  github_runner_id: null,
  gitlab_runner_id: "41",
  state: "Destroyed",
  subphase: null,
  association_status: "verified",
  created_at: 1800000000000,
  updated_at: 1800000009000,
};
const finished = {
  ...job,
  observed_status: "completed",
  reported_result: "success",
  freshness: "confirmed",
  gitlab: {
    ...job.gitlab,
    last_reported_status: "success",
    last_observed_at: 1800000008000,
    result: { status: "success", observed_at: 1800000008000 },
  },
};

async function history(page: Page) {
  await mockApi(page, ["fleet.read", "logs.read"]);
  await page.route("**/api/v1/jobs**", (route) => {
    const path = new URL(route.request().url()).pathname;
    return route.fulfill({
      json: path.endsWith("gitlab-job")
        ? {
            ...finished,
            generations: [generation],
            observations: [],
            observations_truncated: false,
            gitlab_observations: [
              {
                id: "read",
                reported_status: "success",
                source: "job_read",
                observed_at: 1800000008000,
              },
              {
                id: "claim",
                reported_status: "running",
                source: "runner_jobs",
                runner_id: "41",
                observed_at: 1800000001000,
              },
            ],
          }
        : { items: [job], next_cursor: null },
    });
  });
  await page.route("**/api/v1/generations**", (route) => {
    const path = new URL(route.request().url()).pathname;
    return route.fulfill({
      json: path.endsWith("invocations")
        ? { items: [], next_cursor: null, latest_create: null, latest_destroy: null }
        : { ...generation, jobs: [finished] },
    });
  });
}

test("GitLab jobs show the backend, pipeline and exact decimal identity", async ({ page }) => {
  await history(page);
  await page.goto("/jobs?fleet_key=gitlab-build");
  const row = page.getByRole("row").filter({ hasText: "match" });
  await expect(row).toContainText("GitLab / verified association");
  await expect(row).toContainText("platform/ci/app");
  await expect(row).toContainText("Pipeline 77, job 9007199254740993");
  await expect(row).toContainText("Job read fresh");
  await expect(row).not.toContainText("GitHub");
});

test("a finished GitLab job keeps its Verified runner after registration deletion", async ({
  page,
}) => {
  await history(page);
  await page.goto("/jobs/gitlab-job");
  await expect(page.getByRole("heading", { name: "GitLab CI job" })).toBeVisible();
  await expect(page.getByText("Job result confirmed", { exact: true })).toBeVisible();
  await expect(page.getByText("Job read: success", { exact: true })).toBeVisible();
  await expect(page.getByText("Runner jobs: running", { exact: true })).toBeVisible();
  await expect(page.getByText("No verified runner association", { exact: true })).toHaveCount(0);
  await expect(page.getByRole("link", { name: "gitlab-build-1", exact: true })).toHaveAttribute(
    "href",
    "/jobs/runners/gitlab-gen-1",
  );
  await expect(page.getByRole("link", { name: "Open GitLab job", exact: true })).toHaveAttribute(
    "href",
    "https://gitlab.example.test/platform/ci/app/-/jobs/9007199254740993",
  );
  await expect(page.getByText("GitHub conclusion", { exact: true })).toHaveCount(0);
  await expect(page.getByText("Listener freshness", { exact: true })).toHaveCount(0);
  for (const [name, width] of [
    ["desktop", 1280],
    ["mobile", 390],
  ] as const) {
    await page.setViewportSize({ width, height: 900 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
      true,
    );
    await page.screenshot({ path: `test-results/gitlab-job-${name}.png`, fullPage: true });
  }

  await page.getByRole("link", { name: "gitlab-build-1", exact: true }).click();
  await expect(page.getByText("GitLab runner ID", { exact: true })).toBeVisible();
  await expect(page.getByRole("link", { name: "match", exact: true })).toBeVisible();
});

test("an off-instance GitLab job link is never rendered", async ({ page }) => {
  await history(page);
  await page.route("**/api/v1/jobs/gitlab-job", (route) =>
    route.fulfill({
      json: {
        ...job,
        gitlab: { ...job.gitlab, web_url: "https://evil.example.test/platform/ci/app/-/jobs/1" },
        generations: [generation],
        observations: [],
        observations_truncated: false,
        gitlab_observations: [],
      },
    }),
  );
  await page.goto("/jobs/gitlab-job");
  await expect(page.getByRole("heading", { name: "GitLab CI job" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Open GitLab job" })).toHaveCount(0);
});
