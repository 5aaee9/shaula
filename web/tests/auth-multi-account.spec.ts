import { expect, test } from "@playwright/test";
import { mockApi, scopes } from "./fixtures";

import { LEGACY_APP_PROFILE, V2_PROFILE } from "./auth-fixtures";

test("new GitHub App profiles use multi-account selectors without installation input", async ({
  page,
}) => {
  await mockApi(page);
  await page.goto("/auth");
  await page.getByRole("button", { name: "Create profile" }).click();
  await page.getByRole("textbox", { name: "Profile key", exact: true }).fill("shared-github");
  await page.getByLabel("Credential type").selectOption("github_app");
  // No installation field: the App discovers installations per account.
  await expect(page.getByLabel("Installation ID")).toHaveCount(0);
  await page.getByLabel("App ID", { exact: true }).fill("4863460");
  await page.getByLabel("Private key (PEM)").fill("-----BEGIN TEST-----");
  await page.getByLabel("Selector type").selectOption("account_repositories");
  await page.getByLabel("Account type").selectOption("user");
  await page.getByLabel("Owner").fill("5aaee9");

  let submitted: string | null = null;
  await page.route("**/api/v1/github-auth-profiles/shared-github", async (route) => {
    expect(route.request().method()).toBe("PUT");
    if (route.request().method() !== "PUT") return route.fallback();
    submitted = route.request().postData();
    return route.fulfill({
      status: 202,
      contentType: "application/json",
      body: JSON.stringify({ changeId: "c1", state: "Pending", revision: 1 }),
    });
  });
  await page.getByRole("button", { name: "Create profile" }).click();
  await expect.poll(() => submitted).not.toBeNull();
  const body = JSON.parse(submitted!);
  expect(body.schema_version).toBe(2);
  expect(body.installation_id).toBeUndefined();
  expect(body.target_policy).toEqual([
    { kind: "account_repositories", account_kind: "user", owner: "5aaee9" },
  ]);
});

test("legacy App rotation keeps the immutable exact scope and requires the installation", async ({
  page,
}) => {
  await mockApi(page);
  let submitted: string | null = null;
  await page.route("**/api/v1/github-auth-profiles/legacy-app", async (route) => {
    if (route.request().method() === "PUT") {
      submitted = route.request().postData();
      return route.fulfill({
        status: 202,
        contentType: "application/json",
        body: JSON.stringify({ changeId: "c2", state: "Pending", revision: 2 }),
      });
    }
    return route.fulfill({
      headers: { etag: '"auth-inc:1"' },
      json: LEGACY_APP_PROFILE,
    });
  });
  await page.goto("/auth?key=legacy-app");
  await page.getByRole("button", { name: "Rotate credential" }).click();
  // The immutable legacy scope is preserved and prefilled.
  const targets = page.getByLabel("Allowed targets (immutable exact scope)");
  await expect(targets).toHaveValue("acme\nacme/build-tools");
  // Identity members are prefilled from the parsed legacy identity.
  await expect(page.getByLabel("Installation ID")).toHaveValue("34");
  // No upgrade submitted by default: opening the form never expands scope.
  await page.getByLabel("Private key (PEM)").fill("-----BEGIN TEST-----");
  await page.getByRole("dialog").getByRole("button", { name: "Rotate credential" }).click();
  await expect.poll(() => submitted).not.toBeNull();
  const body = JSON.parse(submitted!);
  expect(body.schema_version).toBeUndefined();
  expect(body.app_id).toBe("Iv23legacy");
  expect(body.installation_id).toBe(34);
  expect(body.target_allowlist).toEqual([
    { kind: "organization", owner: "acme" },
    { kind: "repository", owner: "acme", repository: "build-tools" },
  ]);
});

test("legacy profiles expose an explicit upgrade path seeded from the exact scope", async ({
  page,
}) => {
  await mockApi(page);
  let submitted: string | null = null;
  await page.route("**/api/v1/github-auth-profiles/legacy-app", async (route) => {
    if (route.request().method() === "PUT") {
      submitted = route.request().postData();
      return route.fulfill({
        status: 202,
        contentType: "application/json",
        body: JSON.stringify({ changeId: "c3", state: "Pending", revision: 2 }),
      });
    }
    return route.fulfill({
      headers: { etag: '"auth-inc:1"' },
      json: LEGACY_APP_PROFILE,
    });
  });
  await page.goto("/auth?key=legacy-app");
  await page.getByRole("button", { name: "Rotate credential" }).click();
  await page.getByLabel("Upgrade to multi-account policy").check();
  // The legacy installation input disappears once the upgrade mode is on.
  await expect(page.getByLabel("Installation ID")).toHaveCount(0);
  // The legacy client-ID is NOT a valid v2 App id: the numeric id of the
  // SAME App must be entered (continuity proven through /app later).
  await expect(page.getByLabel("App ID")).toHaveValue("Iv23legacy");
  expect(
    await page
      .getByLabel("App ID")
      .evaluate((input) => (input as HTMLInputElement).validity.patternMismatch),
  ).toBe(true);
  await page.getByLabel("App ID").fill("4863460");
  await page.getByLabel("Private key (PEM)").fill("-----BEGIN TEST-----");
  await page.getByRole("dialog").getByRole("button", { name: "Rotate credential" }).click();
  await expect.poll(() => submitted).not.toBeNull();
  const body = JSON.parse(submitted!);
  expect(body.schema_version).toBe(2);
  expect(body.app_id).toBe("4863460");
  expect(body.target_policy).toEqual([
    { kind: "organization", owner: "acme" },
    { kind: "repository", owner: "acme", repository: "build-tools" },
  ]);
});

test("a v2 profile shows its active policy, candidate policy and per-account bindings", async ({
  page,
}) => {
  await mockApi(page);
  await page.route("**/api/v1/github-auth-profiles/shared-github", (route) =>
    route.fulfill({
      headers: { etag: '"auth-inc:2"' },
      json: V2_PROFILE,
    }),
  );
  await page.goto("/auth?key=shared-github");
  await expect(page.getByText("Indexyz (org runners)").first()).toBeVisible();
  await expect(page.getByText("5aaee9 (user repositories)").first()).toBeVisible();
  await expect(page.getByText("Candidate r2: Validating")).toBeVisible();
  await expect(page.getByText("Account bindings of r1")).toBeVisible();
  await expect(page.getByText("All repositories (includes future)")).toBeVisible();
  await expect(page.getByText("Unknown", { exact: true })).toBeVisible();
  // The compose form never shows a single installation identity for v2.
  await expect(page.getByText("app/Iv23legacy/installation/34")).toHaveCount(0);
});

test("all auth surfaces need the auth.write scope to publish", async ({ page }) => {
  const readOnly = scopes.filter((scope) => scope !== "auth.write");
  await mockApi(page, readOnly);
  await page.goto("/auth?key=shared-github");
  await expect(page.getByRole("button", { name: "Rotate credential" })).toBeDisabled();
});
