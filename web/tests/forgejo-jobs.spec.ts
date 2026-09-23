import { expect, test } from "@playwright/test";
import { mockApi } from "./fixtures";

const job = {
  id: "forgejo-job",
  backend: "forgejo",
  fleet_key: "forgejo-build",
  protocol_job_id: "9:7:2",
  owner_name: null,
  repository_name: null,
  job_display_name: "Forgejo build",
  observed_status: "running",
  reported_result: null,
  association_status: "unverified",
  freshness: "stale",
  created_at: 1800000000000,
  updated_at: 1800000001000,
  forgejo: {
    target: { instance_url: "https://forgejo.example.test", scope: { kind: "instance" } },
    repository_id: "18446744073709551615",
    job_id: "7",
    attempt: "2",
    run_id: "8",
    task_id: "42",
    runs_on: ["linux"],
    last_reported_status: "running",
    last_observed_at: 1800000001000,
    in_snapshot: true,
  },
};

test("Forgejo jobs show the backend, exact identity and stale snapshot without a verified association", async ({
  page,
}) => {
  await mockApi(page);
  await page.route("**/api/v1/jobs**", (route) =>
    route.fulfill({ json: { items: [job], next_cursor: null } }),
  );
  await page.goto("/jobs?fleet_key=forgejo-build");
  const row = page.getByRole("row").filter({ hasText: "Forgejo build" });
  await expect(row).toContainText("Forgejo / unverified association");
  await expect(row).toContainText("Snapshot stale");
  await expect(row).toContainText("18446744073709551615");
  await expect(row).toContainText("Run 8, attempt 2");
});

for (const conclusion of ["success", "failure", "cancelled", "skipped"]) {
  test(`Forgejo exact task history shows ${conclusion} without a verified runner`, async ({
    page,
  }) => {
    await mockApi(page);
    await page.route("**/api/v1/jobs/forgejo-job", (route) =>
      route.fulfill({
        json: {
          ...job,
          observed_status: "completed",
          reported_result: conclusion,
          freshness: "confirmed",
          forgejo: {
            ...job.forgejo,
            in_snapshot: false,
            result: {
              task_id: "42",
              conclusion,
              observed_at: 1800000002000,
              run_number: "8",
              run_url: "https://forgejo.example.test/owner/repo/actions/runs/8",
              workflow: "build.yml",
            },
          },
          generations: [],
          observations: [],
          observations_truncated: false,
          forgejo_observations: [
            {
              id: "final",
              reported_status: conclusion,
              task_id: "42",
              observed_at: 1800000002000,
              source: "task_history",
            },
          ],
        },
      }),
    );
    await page.goto("/jobs/forgejo-job");
    await expect(page.getByText("Task result", { exact: true })).toBeVisible();
    await expect(page.getByText(`Task history: ${conclusion}`, { exact: true })).toBeVisible();
    await expect(page.getByText("No verified runner association", { exact: true })).toBeVisible();
    await expect(
      page.getByRole("link", { name: "Open workflow run", exact: true }),
    ).toHaveAttribute("href", "https://forgejo.example.test/owner/repo/actions/runs/8");
    await expect(page.getByText("GitHub conclusion", { exact: true })).toHaveCount(0);
    for (const width of [1280, 390]) {
      await page.setViewportSize({ width, height: 900 });
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
        true,
      );
    }
    if (conclusion === "success")
      await page.screenshot({ path: "test-results/forgejo-job-result-mobile.png", fullPage: true });
  });
}

test("a no-longer-listed Forgejo job has an unknown outcome and provider-specific observations", async ({
  page,
}) => {
  await mockApi(page);
  await page.route("**/api/v1/jobs/forgejo-job", (route) =>
    route.fulfill({
      json: {
        ...job,
        observed_status: "unknown",
        freshness: "not_listed",
        forgejo: { ...job.forgejo, in_snapshot: false },
        generations: [],
        observations: [],
        observations_truncated: false,
        forgejo_observations: [
          { id: "gone", reported_status: null, task_id: "42", observed_at: 1800000002000 },
          { id: "running", reported_status: "running", task_id: "42", observed_at: 1800000001000 },
          { id: "waiting", reported_status: "waiting", task_id: "0", observed_at: 1800000000000 },
        ],
      },
    }),
  );
  await page.goto("/jobs/forgejo-job");
  await expect(page.getByRole("heading", { name: "Forgejo workflow job" })).toBeVisible();
  await expect(page.getByText("No longer listed; outcome unknown", { exact: true })).toBeVisible();
  await expect(page.getByText("GitHub conclusion", { exact: true })).toHaveCount(0);
  await expect(page.getByText("Listener freshness", { exact: true })).toHaveCount(0);
  await expect(page.getByText("No verified runner association", { exact: true })).toBeVisible();
  await expect(page.getByRole("link", { name: "Open workflow run" })).toHaveCount(0);
  for (const [name, width] of [
    ["desktop", 1280],
    ["mobile", 390],
  ] as const) {
    await page.setViewportSize({ width, height: 900 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
      true,
    );
    await page.screenshot({ path: `test-results/forgejo-jobs-${name}.png`, fullPage: true });
  }
});
