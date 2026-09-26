import { expect, test } from "@playwright/test";
import { mockApi } from "./fixtures";

function report(freshness = "fresh") {
  return {
    schemaVersion: 1,
    subject: { kind: "fleet", key: "linux-build", fleetIncarnation: "fleet-incarnation" },
    generatedAt: "2026-09-26T00:00:10.000Z",
    related: [],
    truncated: false,
    questions: [
      {
        question: "scale_up",
        outcome: "blocked",
        coverage: "partial",
        basis: {
          kind: "recorded_decision",
          observationId: "epoch:2",
          observedAt: "2026-09-26T00:00:00.000Z",
          validUntil: "2026-09-26T00:00:30.000Z",
          freshness,
          subjectRevision: "2",
        },
        stages: [
          { id: "capacity", evaluation: "blocked", reasonIds: ["r1"] },
          { id: "pool", evaluation: "not_evaluated", reasonIds: [] },
        ],
        primaryReasonId: "r1",
        reasons: [
          {
            id: "r1",
            code: "capacity.occupancy_limit",
            stage: "capacity",
            severity: "warning",
            effect: "blocking",
            parameters: {},
            evidenceIds: ["e1"],
          },
        ],
        evidence: [
          {
            id: "e1",
            kind: "derived_calculation",
            observedAt: "2026-09-26T00:00:00.000Z",
            freshness,
          },
        ],
        suggestions: [],
        capacity: {
          target: "7",
          demand: "9007199254740993",
          occupancy: "10",
          arithmeticCreateAllowance: "0",
          actuallyAdmitted: null,
        },
      },
    ],
  };
}
test("Why explains actual occupancy and exact counts without executing suggestions", async ({
  page,
}) => {
  await mockApi(page);
  let reads = 0;
  await page.route("**/api/v1/fleets/linux-build/diagnostics", async (route) => {
    expect(route.request().method()).toBe("GET");
    reads++;
    await route.fulfill({ json: report() });
  });
  await page.goto("/fleets/linux-build");
  const panel = page.getByRole("region", { name: "Why", exact: true });
  await expect(panel.getByText("Blocked", { exact: true })).toBeVisible();
  await expect(
    panel.getByText("Resource occupancy leaves no create headroom", { exact: true }).first(),
  ).toBeVisible();
  await panel.getByText("Evidence and evaluated stages").click();
  await expect(panel.getByText("9007199254740993", { exact: true })).toBeVisible();
  await expect(panel.getByText("pool: not_evaluated", { exact: true })).toBeVisible();
  await panel.getByRole("button", { name: "Why" }).click();
  const before = reads;
  await page.waitForTimeout(5500);
  expect(reads).toBe(before);
});
test("stale and future outcome are displayed conservatively", async ({ page }) => {
  await mockApi(page);
  await page.route("**/api/v1/fleets/linux-build/diagnostics", (route) =>
    route.fulfill({ json: report("stale") }),
  );
  await page.goto("/fleets/linux-build");
  const panel = page.getByRole("region", { name: "Why", exact: true });
  await expect(panel.getByText("Unknown", { exact: true }).first()).toBeVisible();
  await expect(panel.getByText("Blocked", { exact: true })).toHaveCount(0);
  await expect(panel.getByText(/observed 2026-09-26T00:00:00.000Z/)).toBeVisible();
});
test("old server fallback does not report that the Fleet was deleted", async ({ page }) => {
  await mockApi(page);
  await page.route("**/api/v1/fleets/linux-build/diagnostics", (route) =>
    route.fulfill({ contentType: "text/html", body: "<html>old SPA fallback</html>" }),
  );
  await page.goto("/fleets/linux-build");
  await expect(
    page.getByText(
      "This server does not support diagnostics. The existing status remains available.",
    ),
  ).toBeVisible();
  await expect(page.getByRole("heading", { name: "linux-build", exact: true })).toBeVisible();
});
test("unknown reason is literal text and cannot inject markup", async ({ page }) => {
  await mockApi(page);
  const data = report();
  data.questions[0].reasons[0].code = '<img src=x onerror="window.injected=true">';
  await page.route("**/api/v1/fleets/linux-build/diagnostics", (route) =>
    route.fulfill({ json: data }),
  );
  await page.goto("/fleets/linux-build");
  const panel = page.getByRole("region", { name: "Why", exact: true });
  await expect(
    panel.getByText(/The current client does not recognize this reason/).first(),
  ).toBeVisible();
  await expect(panel.locator("img")).toHaveCount(0);
});

test("unsupported schema and wrong incarnation cannot become current explanations", async ({
  page,
}) => {
  await mockApi(page);
  await page.route("**/api/v1/fleets/linux-build/diagnostics", (route) =>
    route.fulfill({ json: { schemaVersion: 2 } }),
  );
  await page.goto("/fleets/linux-build");
  await expect(page.getByText(/This server does not support diagnostics/)).toBeVisible();
  await page.unroute("**/api/v1/fleets/linux-build/diagnostics");
  const wrong = report();
  wrong.subject.fleetIncarnation = "old-incarnation";
  await page.route("**/api/v1/fleets/linux-build/diagnostics", (route) =>
    route.fulfill({ json: wrong }),
  );
  await page.reload();
  const panel = page.getByRole("region", { name: "Why", exact: true });
  await expect(panel.getByText(/temporarily unavailable/)).toBeVisible();
  await expect(panel.getByText("Blocked", { exact: true })).toHaveCount(0);
});

test("no read permission starts no diagnostics requests", async ({ page }) => {
  await mockApi(page, ["fleet.write"]);
  let reads = 0;
  await page.route("**/diagnostics", (route) => {
    reads++;
    return route.fulfill({ json: report() });
  });
  await page.goto("/fleets/linux-build");
  await expect(page.getByText(/fleet.read|permission/i).first()).toBeVisible();
  expect(reads).toBe(0);
});

test("revoked permission removes retained diagnostic evidence", async ({ page }) => {
  await mockApi(page);
  await page.clock.install();
  let denied = false;
  await page.route("**/api/v1/fleets/linux-build/diagnostics", (route) =>
    !denied
      ? route.fulfill({ json: report() })
      : route.fulfill({ status: 403, json: { code: "Forbidden" } }),
  );
  await page.goto("/fleets/linux-build");
  const panel = page.getByRole("region", { name: "Why", exact: true });
  await expect(panel.getByText("Blocked", { exact: true })).toBeVisible();
  denied = true;
  await page.clock.fastForward(5100);
  await expect(panel.getByText(/no longer have permission/)).toBeVisible();
  await expect(panel.getByText("Blocked", { exact: true })).toHaveCount(0);
});

test("hidden pages stop diagnostic polling", async ({ page }) => {
  await mockApi(page);
  await page.clock.install();
  let reads = 0;
  await page.route("**/api/v1/fleets/linux-build/diagnostics", (route) => {
    reads++;
    return route.fulfill({ json: report() });
  });
  await page.goto("/fleets/linux-build");
  await expect(
    page.getByRole("region", { name: "Why", exact: true }).getByText("Blocked", { exact: true }),
  ).toBeVisible();
  await page.evaluate(() => {
    Object.defineProperty(document, "visibilityState", { configurable: true, value: "hidden" });
    document.dispatchEvent(new Event("visibilitychange"));
  });
  const before = reads;
  await page.clock.fastForward(31_000);
  expect(reads).toBe(before);
});
