import { expect, test } from "@playwright/test";
import { mockApi, scopes } from "./fixtures";

import { UNSUPPORTED_PROFILE, V2_PROFILE } from "./auth-fixtures";

test("new GitHub App profiles use multi-account selectors without installation input", async ({
  page,
}) => {
  await mockApi(page);
  await page.goto("/auth");
  await page.getByRole("button", { name: "Create profile" }).click();
  await page.getByRole("textbox", { name: "Profile key", exact: true }).fill("shared-github");
  await expect(page.getByLabel("Credential type")).toHaveCount(0);
  await expect(page.getByLabel("Personal access token", { exact: true })).toHaveCount(0);
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

test("unsupported authentication remains visible without rotation or upgrade", async ({ page }) => {
  await mockApi(page);
  await page.route("**/api/v1/github-auth-profiles/old-auth", (route) =>
    route.fulfill({ headers: { etag: '"auth-inc:1"' }, json: UNSUPPORTED_PROFILE }),
  );
  await page.goto("/auth?key=old-auth");
  await expect(page.getByText("Unsupported", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Rotate credential" })).toBeDisabled();
  await expect(page.getByLabel("Upgrade to multi-account policy")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Retire authentication profile" })).toBeEnabled();
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
});

test("all auth surfaces need the auth.write scope to publish", async ({ page }) => {
  const readOnly = scopes.filter((scope) => scope !== "auth.write");
  await mockApi(page, readOnly);
  await page.goto("/auth?key=shared-github");
  await expect(page.getByRole("button", { name: "Rotate credential" })).toBeDisabled();
});
