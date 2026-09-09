import { expect, test, type Page } from "@playwright/test";
import { mockApi } from "./fixtures";

const job = {
  id: "job-1",
  fleet_key: "linux-build",
  protocol_job_id: "opaque-job",
  owner_name: "acme",
  repository_name: "widgets",
  job_display_name: "Build Linux",
  job_workflow_ref: "acme/widgets/.github/workflows/build.yml@main",
  workflow_run_id: 42,
  workflow_run_attempt: null,
  observed_status: "running",
  reported_result: null,
  github_conclusion: null,
  github_run_url: "https://github.com/acme/widgets/actions/runs/42",
  association_status: "verified",
  freshness: "unknown",
  created_at: 1800000000000,
  updated_at: 1800000001000,
};
const generation = {
  id: "gen-1",
  fleet_key: "linux-build",
  runner_name: "shaula-linux-1",
  generation_name: "gen-1",
  github_runner_id: 314,
  state: "Busy",
  subphase: null,
  association_status: "verified",
  created_at: 1800000000000,
  updated_at: 1800000001000,
};
const attempts = [
  {
    id: "destroy-2",
    generation_id: "gen-1",
    operation: "Destroy",
    ordinal: 2,
    started_at: 1800000005000,
    ended_at: null,
    execution_outcome: "failed",
    capture_status: "partial",
    retained_bytes: 42,
    lost_bytes: 10,
    commands: [{ phase: "destroy", termination: "failed", exit_code: 1 }],
  },
  {
    id: "apply-1",
    generation_id: "gen-1",
    operation: "Create",
    ordinal: 1,
    started_at: 1800000000000,
    ended_at: 1800000003000,
    execution_outcome: "succeeded",
    capture_status: "complete",
    retained_bytes: 64,
    lost_bytes: 0,
    commands: [{ phase: "apply", termination: "exited", exit_code: 0 }],
  },
];
async function history(page: Page, permissions = ["fleet.read", "logs.read"]) {
  await mockApi(page, permissions);
  await page.route("**/api/v1/jobs**", (route) => {
    const path = new URL(route.request().url()).pathname;
    return route.fulfill({
      json: path.endsWith("job-1")
        ? { ...job, observations: [], generations: [generation] }
        : { items: [job], next_cursor: null },
    });
  });
  await page.route("**/api/v1/generations**", (route) => {
    const path = new URL(route.request().url()).pathname;
    return route.fulfill({
      json: path.endsWith("invocations")
        ? {
            items: attempts,
            next_cursor: null,
            latest_create: attempts[1],
            latest_destroy: attempts[0],
          }
        : path.endsWith("gen-1")
          ? { ...generation, jobs: [job] }
          : { items: [generation], next_cursor: null },
    });
  });
  await page.route("**/api/v1/invocations/**/logs**", (route) => {
    const url = new URL(route.request().url());
    const id = url.pathname.split("/")[4];
    const next = url.searchParams.has("cursor");
    return route.fulfill({
      json: {
        invocation_id: id,
        content_version: "v1",
        capture_status: id === "apply-1" ? "complete" : "partial",
        entries: [
          {
            command_ordinal: 1,
            phase: "apply",
            stream: "stdout",
            sequence: 1,
            text: next
              ? "Next retained page"
              : id === "apply-1"
                ? "Apply complete. <script>window.logExecuted=true</script>\n"
                : "Destroy failed: resource busy\n",
            observed_at: 1800000001000,
          },
        ],
        next_cursor: next ? null : "cursor-2",
        has_gap: id !== "apply-1",
        lost_bytes: 10,
      },
    });
  });
}

test("Jobs navigate to workflow history, keep separate outcomes, and render bounded text safely", async ({
  page,
}) => {
  await history(page);
  await page.goto("/jobs");
  await page.getByRole("link", { name: "Build Linux", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Build Linux" })).toBeVisible();
  await expect(page.getByText("Run attempt", { exact: true }).locator("..")).toContainText(
    "Unknown",
  );
  await expect(page.getByRole("link", { name: "Open workflow run" })).toHaveAttribute(
    "href",
    job.github_run_url,
  );
  await expect(page.getByLabel("Operation log")).toContainText("Destroy failed");
  await page.getByLabel("Execution attempt").selectOption("apply-1");
  await expect(page.getByLabel("Operation log")).toContainText("<script>");
  expect(await page.evaluate(() => "logExecuted" in window)).toBe(false);
  await page.getByRole("button", { name: "Next log page" }).click();
  await expect(page.getByLabel("Operation log")).toHaveText("Next retained page");
  await page.getByLabel("Log stream").selectOption("stderr");
  await expect(page.getByLabel("Operation log")).toContainText("Apply complete");
  await page.screenshot({ path: "test-results/jobs-detail.png", fullPage: true });
});

test("fleet.read alone shows invocation status without requesting log bodies", async ({ page }) => {
  await history(page, ["fleet.read"]);
  const requests: string[] = [];
  page.on("request", (request) => {
    if (request.url().includes("/logs")) requests.push(request.url());
  });
  await page.goto("/jobs/runners/gen-1");
  await expect(page.getByText("logs.read permission is required to view log text.")).toBeVisible();
  await expect(page.getByText("failed", { exact: true }).first()).toBeVisible();
  expect(requests).toEqual([]);
});

test("unassigned runners retain fleet filter and remain usable on mobile", async ({ page }) => {
  await history(page);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/jobs/runners?fleet_key=linux-build");
  await expect(page.getByLabel("Filter by fleet")).toHaveValue("linux-build");
  await page.getByRole("link", { name: "shaula-linux-1" }).click();
  await expect(page.getByRole("heading", { name: "shaula-linux-1" })).toBeVisible();
  await expect
    .poll(() => page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth))
    .toBe(true);
});

test("authentication expiry removes retained log text", async ({ page }) => {
  await history(page);
  await page.goto("/jobs/runners/gen-1");
  await expect(page.getByLabel("Operation log")).toContainText("Destroy failed");
  await page.route("**/api/v1/invocations/**/logs**", (route) =>
    route.fulfill({
      status: 401,
      json: { code: "Unauthenticated", detail: "Authentication required" },
    }),
  );
  await page.getByLabel("Log stream").selectOption("stderr");
  await expect(page.getByRole("heading", { name: "Sign in to Shaula" })).toBeVisible();
  await expect(page.getByText("Destroy failed", { exact: false })).toHaveCount(0);
});

test("unavailable invocation history is distinct from an empty archive", async ({ page }) => {
  await history(page);
  await page.route("**/api/v1/generations/gen-1/invocations**", (route) =>
    route.fulfill({
      status: 503,
      json: { code: "HistoryUnavailable", detail: "History is temporarily unavailable" },
    }),
  );
  await page.goto("/jobs/runners/gen-1");
  await expect(page.getByRole("alert")).toContainText("History is temporarily unavailable");
  await expect(page.getByText("No recorded invocations")).toHaveCount(0);
});

test("ambiguous candidate jobs are visibly unconfirmed", async ({ page }) => {
  await history(page);
  await page.route("**/api/v1/generations/gen-1", (route) =>
    route.fulfill({
      json: {
        ...generation,
        association_status: "ambiguous",
        jobs: [{ ...job, association_status: "ambiguous" }],
      },
    }),
  );
  await page.goto("/jobs/runners/gen-1");
  await expect(page.getByText("Job association", { exact: true }).locator("..")).toContainText(
    "ambiguous",
  );
  await expect(
    page.getByText("No workflow job is confirmed for this runner.", { exact: false }),
  ).toBeVisible();
});

test("older attempt pages keep current Apply and Destroy outcomes", async ({ page }) => {
  await history(page);
  await page.route("**/api/v1/generations/gen-1/invocations**", (route) => {
    const older = new URL(route.request().url()).searchParams.has("cursor");
    return route.fulfill({
      json: {
        items: older ? [attempts[1]] : [attempts[0]],
        next_cursor: older ? null : "older",
        latest_create: attempts[1],
        latest_destroy: attempts[0],
      },
    });
  });
  await page.goto("/jobs/runners/gen-1");
  await page.getByRole("button", { name: "Older attempts" }).click();
  await expect(page.getByLabel("Execution attempt")).toHaveValue("apply-1");
  await expect(page.getByText("failed", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Latest attempts" }).click();
  await expect(page.getByLabel("Execution attempt")).toHaveValue("destroy-2");
});
