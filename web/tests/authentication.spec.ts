import { expect, test } from "@playwright/test";
import { mockApi } from "./fixtures";

test("session expiry unmounts credential forms and clears browser state", async ({ page }) => {
  await mockApi(page);
  await page.goto("/auth?key=github-build");
  await page.getByRole("button", { name: "Rotate credential" }).click();
  await page.getByLabel("Private key (PEM)", { exact: true }).fill("draft-secret");
  await page.route("**/readyz", (route) =>
    route.fulfill({
      status: 401,
      contentType: "application/problem+json",
      body: JSON.stringify({ code: "AuthenticationFailed" }),
    }),
  );
  await expect(page.getByRole("heading", { name: "Sign in to Shaula" })).toBeVisible({
    timeout: 15000,
  });
  await expect(page.getByLabel("Private key (PEM)", { exact: true })).toHaveCount(0);
  expect(
    await page.evaluate(() => [Object.keys(localStorage), Object.keys(sessionStorage)]),
  ).toEqual([[], []]);
});

test("logout sends the session CSRF token and clears the resource view", async ({ page }) => {
  await mockApi(page);
  await page.route("**/auth/oidc/logout", (route) => {
    expect(route.request().method()).toBe("POST");
    expect(route.request().headers()["x-csrf-token"]).toBe("test-session-csrf");
    return route.fulfill({ status: 204 });
  });
  await page.goto("/fleets");
  await expect(page.getByText("linux-build", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Session menu" }).click();
  await page.getByRole("menuitem", { name: "Sign out", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Sign in to Shaula" })).toBeVisible();
  await expect(page.getByText("linux-build", { exact: true })).toHaveCount(0);
});
