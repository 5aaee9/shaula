import { expect, test } from "@playwright/test";
import { mockApi } from "./fixtures";

const permissions = [
  "fleet.read",
  "auth.write",
  "access-token.read",
  "access-token.write",
  "access-token.revoke",
];
const principal = { issuer: "https://oidc.example", subject: "operator" };
const token = {
  id: "a".repeat(32),
  name: "Build reader",
  scopes: ["fleet.read"],
  effective_scopes: ["fleet.read"],
  created_at: "2026-09-25T00:00:00Z",
  expires_at: "2026-10-25T00:00:00Z",
  last_used_at: null,
  revoked_at: null,
  state: "active",
  revision: 1,
};
const secret = `shaula_pat_v1_${token.id}_${"b".repeat(43)}`;

test("disabled policy retains metadata management but prevents issuance", async ({ page }) => {
  await mockApi(page, permissions);
  await page.route("**/api/v1/session", (route) =>
    route.fulfill({
      json: {
        name: "Operator",
        scopes: permissions,
        capabilities: { personal_access_tokens: false, token_management_api: 1 },
      },
    }),
  );
  await page.route("**/api/v1/access-tokens**", (route) =>
    route.fulfill({ json: { items: [{ ...token, state: "disabled" }], next_cursor: null } }),
  );
  await page.goto("/settings/access-tokens");
  await expect(
    page.getByText("Personal access tokens are disabled by the deployment.", { exact: false }),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "Create token", exact: true })).toHaveCount(0);
  await expect(
    page.getByRole("heading", { name: `${token.name} (disabled)`, exact: true }),
  ).toBeVisible();
});

test("issuance has no default scopes, reveals once, and clears secret on navigation", async ({
  page,
}) => {
  await mockApi(page, permissions);
  await page.route("**/api/v1/access-tokens**", (route) => {
    if (route.request().method() === "POST") {
      expect(route.request().postDataJSON().scopes).toEqual(["fleet.read"]);
      expect(route.request().headers()["idempotency-key"]).toBeTruthy();
      return route.fulfill({
        status: 201,
        json: { access_token: token, secret_available: true, token: secret },
      });
    }
    return route.fulfill({ json: { items: [], next_cursor: null } });
  });
  await page.goto("/settings/access-tokens");
  await expect(page.getByRole("checkbox", { checked: true })).toHaveCount(0);
  await expect(page.getByRole("checkbox", { name: "access-token.write", exact: true })).toHaveCount(
    0,
  );
  await page.getByLabel("Name", { exact: true }).fill("Build reader");
  await page.getByRole("checkbox", { name: "fleet.read", exact: true }).check();
  await page.getByRole("button", { name: "Create token", exact: true }).click();
  await expect(page.getByText(secret, { exact: true })).toHaveCount(0);
  await page.getByRole("button", { name: "Reveal token" }).click();
  await expect(page.getByText(secret, { exact: true })).toBeVisible();
  expect(await page.evaluate(() => JSON.stringify([localStorage, sessionStorage]))).not.toContain(
    secret,
  );
  await page.getByRole("link", { name: "Fleets", exact: true }).click();
  await page.getByRole("link", { name: "Access tokens", exact: true }).click();
  await expect(page.getByText(secret, { exact: true })).toHaveCount(0);
});

test("uncertain issuance reuses its key and handles secretless recovery", async ({ page }) => {
  await mockApi(page, permissions);
  const keys: string[] = [];
  await page.route("**/api/v1/access-tokens**", (route) => {
    if (route.request().method() !== "POST")
      return route.fulfill({ json: { items: [], next_cursor: null } });
    keys.push(route.request().headers()["idempotency-key"]);
    if (keys.length === 1)
      return route.fulfill({ status: 503, json: { code: "Unavailable", detail: "Response lost" } });
    return route.fulfill({ json: { access_token: token, secret_available: false } });
  });
  await page.goto("/settings/access-tokens");
  await page.getByLabel("Name", { exact: true }).fill("Build reader");
  await page.getByRole("button", { name: "Create token", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("Retry unchanged input");
  await page.getByRole("button", { name: "Create token", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Request recovered — secret unavailable" }),
  ).toBeVisible();
  expect(keys).toHaveLength(2);
  expect(keys[0]).toBe(keys[1]);
  await expect(page.getByRole("button", { name: "Reveal token" })).toHaveCount(0);
});

test("rotation verifies saved replacement before enabling explicit old-token revocation", async ({
  page,
}) => {
  await mockApi(page, permissions);
  const replacement = { ...token, id: "c".repeat(32), name: "Build reader replacement" };
  let verified = false;
  let revoked = false;
  await page.route("**/api/v1/session", (route) => {
    const bearer = route.request().headers().authorization;
    if (bearer) {
      verified = true;
      expect(bearer).toBe(`Bearer ${secret}`);
    }
    return route.fulfill({
      headers: { "x-csrf-token": "test-session-csrf" },
      json: {
        name: "Operator",
        principal,
        scopes: permissions,
        authentication: bearer
          ? { kind: "personal_access_token", token_id: replacement.id }
          : { kind: "oidc_session" },
      },
    });
  });
  await page.route("**/api/v1/access-tokens**", (route) => {
    if (route.request().method() === "DELETE") {
      expect(verified).toBe(true);
      revoked = true;
      expect(route.request().headers()["if-match"]).toBe('"opaque-reviewed-version"');
      return route.fulfill({ status: 204 });
    }
    if (route.request().method() === "POST")
      return route.fulfill({
        status: 201,
        json: { access_token: replacement, secret_available: true, token: secret },
      });
    if (new URL(route.request().url()).pathname.endsWith(token.id))
      return route.fulfill({ headers: { etag: '"opaque-reviewed-version"' }, json: token });
    return route.fulfill({ json: { items: [token], next_cursor: null } });
  });
  await page.goto("/settings/access-tokens");
  await page.getByRole("button", { name: "Rotate", exact: true }).click();
  await page.getByRole("button", { name: "Create token", exact: true }).click();
  await expect(page.getByRole("button", { name: /Revoke old token/ })).toHaveCount(0);
  await page.getByRole("button", { name: "I have saved it — verify" }).click();
  await expect(page.getByRole("heading", { name: "New token saved and verified" })).toBeVisible();
  expect(revoked).toBe(false);
  page.once("dialog", (dialog) => dialog.accept());
  await page.getByRole("button", { name: /Revoke old token/ }).click();
  await expect.poll(() => revoked).toBe(true);
});
