import { expect, test } from "@playwright/test";
import { fleet, mockApi } from "./fixtures";

test("fleet inventory, search, detail navigation and mobile layout", async ({ page }) => {
  await mockApi(page);
  await page.goto("/fleets");
  await expect(page.getByRole("link", { name: "linux-build acme" })).toBeVisible();
  await expect(page.getByText("10", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Collapse navigation" }).click();
  await expect(page.locator('[data-slot="sidebar"]')).toHaveAttribute("data-state", "collapsed");
  await page.getByRole("button", { name: "Expand navigation" }).click();
  await expect(page.locator('[data-slot="sidebar"]')).toHaveAttribute("data-state", "expanded");
  await page.getByRole("button", { name: "Session menu" }).click();
  await expect(page.getByText("fleet.write", { exact: true })).toBeVisible();
  const sessionRefresh = page.waitForResponse("**/api/v1/session");
  await page.getByRole("menuitem", { name: "Refresh session" }).click();
  await sessionRefresh;
  await page.screenshot({
    path: "test-results/fleets-desktop.png",
    fullPage: true,
    animations: "disabled",
  });
  await page.getByRole("textbox", { name: "Search fleets" }).fill("release");
  await expect(page.getByRole("link", { name: "linux-build acme" })).toHaveCount(0);
  await page.getByRole("textbox", { name: "Search fleets" }).fill("");
  await page.getByRole("link", { name: "Open linux-build" }).click();
  await expect(page.getByRole("heading", { name: "linux-build", exact: true })).toBeVisible();
  await page.getByRole("tab", { name: "Desired specification" }).click();
  await expect(page.locator("pre")).toContainText("min_runners");
  await page.reload();
  await expect(page.getByRole("heading", { name: "linux-build", exact: true })).toBeVisible();
  await page.setViewportSize({ width: 390, height: 844 });
  await page.getByRole("button", { name: "Open navigation" }).click();
  await page.getByRole("link", { name: "Fleets", exact: true }).first().click();
  await expect(page.getByRole("heading", { name: "Fleets", exact: true })).toBeVisible();
  await expect(page.getByRole("dialog", { name: "Sidebar" })).toHaveCount(0);
  await expect
    .poll(() => page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth))
    .toBe(true);
  await page.screenshot({
    path: "test-results/fleets-mobile.png",
    fullPage: true,
    animations: "disabled",
  });
});

test("edit captures original ETag and surfaces conflicts without losing input", async ({
  page,
}) => {
  await mockApi(page);
  await page.goto("/fleets/linux-build");
  await page.getByRole("button", { name: "Edit fleet" }).click();
  await page.getByRole("spinbutton", { name: "Maximum runners" }).fill("12");
  await page.route("**/api/v1/fleets/linux-build", async (route) => {
    if (route.request().method() === "GET")
      return route.fulfill({
        headers: { etag: '"fleet-incarnation:3"' },
        json: fleet("linux-build", 3),
      });
    expect(route.request().headers()["if-match"]).toBe('"fleet-incarnation:2"');
    expect(route.request().headers()["idempotency-key"]).toBeTruthy();
    expect(route.request().postDataJSON().capacity.max_runners).toBe(12);
    return route.fulfill({ status: 412, json: { code: "PreconditionFailed", detail: "stale" } });
  });
  // The detail query refreshes behind the open editor.
  await page.waitForResponse(
    (response) =>
      response.url().endsWith("/fleets/linux-build") && response.request().method() === "GET",
    { timeout: 15_000 },
  );
  await page.getByRole("button", { name: "Save changes" }).click();
  await expect(page.getByRole("alert")).toContainText("changed since you opened it");
  await expect(page.getByRole("spinbutton", { name: "Maximum runners" })).toHaveValue("12");
});

test("uncertain writes reuse idempotency and track accepted changes", async ({ page }) => {
  await mockApi(page);
  let firstKey = "";
  let count = 0;
  await page.route("**/api/v1/fleets/linux-build", (route) => {
    if (route.request().method() === "GET")
      return route.fulfill({
        headers: { etag: '"fleet-incarnation:2"' },
        json: fleet("linux-build"),
      });
    const key = route.request().headers()["idempotency-key"];
    if (!count++) {
      firstKey = key;
      return route.abort("failed");
    }
    expect(key).toBe(firstKey);
    return route.fulfill({
      status: 202,
      json: { changeId: "change-1", state: "Accepted", revision: 3 },
    });
  });
  await page.goto("/fleets/linux-build");
  await page.getByRole("button", { name: "Edit fleet" }).click();
  await page.getByRole("button", { name: "Save changes" }).click();
  await expect(page.getByRole("alert")).toBeVisible();
  await page.getByRole("button", { name: "Save changes" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.getByText("Converged", { exact: true })).toBeVisible();
});

test("retirement requires confirmation and sends conditional DELETE", async ({ page }) => {
  await mockApi(page);
  await page.goto("/fleets/linux-build");
  await page.getByRole("button", { name: "Retire fleet" }).click();
  await expect(page.getByRole("button", { name: "Retire", exact: true })).toBeDisabled();
  await page
    .getByRole("textbox", { name: "Confirm resource key: linux-build" })
    .fill("linux-build");
  const request = page.waitForRequest((request) => request.method() === "DELETE");
  await page.route("**/api/v1/fleets/linux-build", (route) => {
    if (route.request().method() === "GET")
      return route.fulfill({
        headers: { etag: '"fleet-incarnation:3"' },
        json: fleet("linux-build", 3),
      });
    return route.fulfill({
      status: 202,
      json: { changeId: "retire-1", state: "Accepted", revision: 3 },
    });
  });
  await page.getByRole("button", { name: "Retire", exact: true }).click();
  expect((await request).headers()["if-match"]).toBe('"fleet-incarnation:2"');
});

test("read-only access hides mutations and authentication failure is explicit", async ({
  page,
}) => {
  await mockApi(page, ["fleet.read", "template.read", "auth.read"]);
  await page.goto("/fleets");
  await expect(page.getByRole("button", { name: "Create fleet" })).toBeDisabled();
  await page.goto("/fleets/linux-build");
  await expect(page.getByRole("button", { name: "Edit fleet" })).toBeDisabled();
  await expect(page.getByRole("button", { name: "Retire fleet" })).toBeDisabled();
  await page.route("**/api/v1/session", (route) =>
    route.fulfill({
      status: 401,
      json: { code: "ActorContextMissing", detail: "missing context" },
    }),
  );
  await page.reload();
  await expect(page.getByRole("heading", { name: "Authentication required" })).toBeVisible();
  await expect(page.getByRole("alert")).toContainText("Authentication required");
});

test("template and authentication profiles use real route contracts", async ({ page }) => {
  await mockApi(page);
  await page.goto("/templates?key=kubernetes-linux");
  await expect(page.getByText("shaula.bindings.kubernetes/v1", { exact: true })).toBeVisible();
  await page.goto("/auth?key=github-build");
  await expect(page.getByText("build-bot", { exact: true })).toBeVisible();
  await expect(page.getByText("Configured", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Rotate credential" }).click();
  await expect(page.getByLabel("Personal access token", { exact: true })).toHaveAttribute(
    "type",
    "password",
  );
  expect(await page.evaluate(() => Object.keys(localStorage))).toEqual([]);
});
