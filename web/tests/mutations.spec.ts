import { expect, test } from "@playwright/test";
import { mockApi } from "./fixtures";

test("new fleet sends the API spec with create precondition", async ({ page }) => {
  await mockApi(page);
  await page.goto("/fleets");
  await page.getByRole("button", { name: "Create fleet" }).click();
  await page.getByLabel("Fleet key", { exact: true }).fill("new-build");
  await page.getByLabel("Owner", { exact: true }).fill("acme");
  await expect(page.getByRole("button", { name: "Advanced settings" })).toHaveAttribute(
    "aria-expanded",
    "false",
  );
  await expect(page.getByLabel("Scale set name", { exact: true })).toBeHidden();
  await page
    .getByLabel("GitHub authentication profile", { exact: true })
    .selectOption("github-build");
  await page.getByLabel("Template profile", { exact: true }).selectOption("kubernetes-linux");
  await expect(page.getByRole("dialog")).toContainText(
    "This template needs no input configuration.",
  );
  await page.screenshot({
    path: "test-results/fleet-dialog-compact.png",
    fullPage: true,
    animations: "disabled",
  });
  await page.route("**/api/v1/fleets/new-build", (route) => {
    const request = route.request();
    expect(request.method()).toBe("PUT");
    expect(request.headers()["if-none-match"]).toBe("*");
    expect(request.headers()["x-csrf-token"]).toBe("test-session-csrf");
    expect(request.headers()["x-shaula-backend-auth"]).toBeUndefined();
    expect(request.headers()["x-shaula-actor"]).toBeUndefined();
    expect(request.postDataJSON().github.target).toEqual({ kind: "organization", owner: "acme" });
    expect(request.postDataJSON().github.auth_profile_ref).toBe("github-build");
    expect(request.postDataJSON().template_profile_ref).toEqual({
      key: "kubernetes-linux",
      revision: 3,
    });
    expect(request.postDataJSON().github.scale_set_name).toBe("new-build");
    expect(request.postDataJSON().github.runner_group).toBe("Default");
    expect(request.postDataJSON().github.labels).toEqual([]);
    expect(request.postDataJSON().capacity).toEqual({ min_runners: 0, max_runners: 10 });
    expect(request.postDataJSON().template_inputs).toEqual({});
    return route.fulfill({
      status: 202,
      json: { changeId: "create-1", state: "Accepted", revision: 1 },
    });
  });
  await page.getByRole("dialog").getByRole("button", { name: "Create fleet" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.getByText("Change for new-build")).toBeVisible();
});

test("auth creation keeps credentials out of storage and the read model", async ({ page }) => {
  await mockApi(page);
  let created = false;
  const profile = {
    key: "new-auth",
    incarnation: "auth-inc",
    kind: "github_app",
    schema_version: 2,
    app_id: "4863460",
    desiredRevision: 1,
    activeRevision: null,
    status: "Validating",
    credential_present: true,
  };
  await page.route("**/api/v1/github-auth-profiles", (route) =>
    route.fulfill({ json: { profiles: created ? [profile] : [] } }),
  );
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/auth");
  await page.getByRole("button", { name: "Create profile" }).click();
  await page.getByLabel("Profile key", { exact: true }).fill("new-auth");
  await page.getByLabel("App ID", { exact: true }).fill("4863460");
  await page.getByLabel("Private key (PEM)", { exact: true }).fill("test-only-secret");
  await page.getByLabel("Owner", { exact: true }).fill("acme");
  await page.route("**/api/v1/github-auth-profiles/new-auth", (route) => {
    if (route.request().method() === "GET") return route.fulfill({ json: profile });
    expect(route.request().headers()["if-none-match"]).toBe("*");
    expect(route.request().postDataJSON()).toEqual({
      kind: "github_app",
      schema_version: 2,
      app_id: "4863460",
      private_key: "test-only-secret",
      target_policy: [{ kind: "organization", owner: "acme" }],
    });
    created = true;
    return route.fulfill({
      status: 202,
      json: { changeId: "auth-1", state: "Accepted", revision: 1 },
    });
  });
  await page.screenshot({ path: "test-results/auth-dialog-mobile.png", fullPage: true });
  expect(
    await page
      .getByRole("dialog")
      .evaluate((element) => element.scrollWidth <= element.clientWidth),
  ).toBe(true);
  await page.getByRole("dialog").getByRole("button", { name: "Create profile" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  const connection = page.getByRole("row").filter({ hasText: "new-auth" });
  await expect(connection.getByText("Validating", { exact: true })).toBeVisible();
  await expect(connection).toHaveAttribute("data-state", "selected");
  expect(await page.evaluate(() => document.body.scrollWidth <= window.innerWidth)).toBe(true);
  expect(await page.evaluate(() => [localStorage.length, sessionStorage.length])).toEqual([0, 0]);
  await expect(page.locator("body")).not.toContainText("test-only-secret");
});

test("empty fleet collection and API failure are distinct", async ({ page }) => {
  await mockApi(page);
  await page.route("**/api/v1/fleets", (route) => route.fulfill({ json: { fleets: [] } }));
  await page.goto("/fleets");
  await expect(page.getByText("No fleets yet", { exact: true })).toBeVisible();
  await page.route("**/api/v1/fleets", (route) =>
    route.fulfill({ status: 503, json: { code: "Unavailable", detail: "Registry unavailable" } }),
  );
  await page.getByRole("button", { name: "Refresh fleets" }).click();
  await expect(page.getByRole("alert")).toContainText("Registry unavailable");
  await expect(page.getByText("No fleets yet", { exact: true })).toHaveCount(0);
});

test("template publication sends only the API-owned inputs", async ({ page }) => {
  await mockApi(page);
  await page.goto("/templates");
  await page.getByRole("button", { name: "Publish template" }).click();
  await page.getByLabel("Profile key", { exact: true }).fill("new-template");
  await expect(page.getByLabel("Bindings (JSON)")).toBeHidden();
  await page.getByRole("radio", { name: "Existing artifact", exact: true }).check();
  await page
    .getByLabel("Existing artifact digest", { exact: true })
    .fill(`sha256:${"a".repeat(64)}`);
  await page.route("**/api/v1/template-profiles/new-template", (route) => {
    if (route.request().method() === "GET") return route.fallback();
    expect(route.request().headers()["if-none-match"]).toBe("*");
    expect(route.request().postDataJSON()).toEqual({
      artifact_digest: `sha256:${"a".repeat(64)}`,
      engine_ref: "terraform",
      bindings: {},
      fleet_input_policy: {},
    });
    return route.fulfill({
      status: 202,
      json: { changeId: "template-1", state: "Accepted", revision: 1 },
    });
  });
  await page.getByRole("button", { name: "Publish", exact: true }).click();
  await expect(page).toHaveURL(/\/templates(?:\?[^#]*)?$/);
  await expect(page.getByRole("form", { name: "Template configuration" })).toHaveCount(0);
  await expect(page.getByText("Change for new-template")).toBeVisible();
});
