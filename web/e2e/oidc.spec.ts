import { expect, test } from "@playwright/test";

test("OIDC login protects embedded bytes and enables session-authorized pages", async ({
  page,
  request,
}) => {
  const anonymous = await request.get("/fleets", { maxRedirects: 0 });
  expect(anonymous.status()).toBe(302);
  expect(await anonymous.text()).not.toContain("<html");
  for (const path of ["/api/v1/session", "/livez", "/assets/missing.js", "/favicon.ico"]) {
    const response = await request.get(path);
    expect(response.status()).toBe(401);
    expect(response.headers()["cache-control"]).toBe("private, no-store");
  }
  await page.goto("/fleets");
  await expect(page.getByRole("heading", { name: "Fleets", exact: true })).toBeVisible();
  await expect(page.getByText("Test operator", { exact: true })).toBeVisible();
  const scripts = await page
    .locator("script[src]")
    .evaluateAll((nodes) => nodes.map((node) => (node as HTMLScriptElement).src));
  expect(scripts.length).toBeGreaterThan(0);
  expect((await request.get(scripts[0])).status()).toBe(401);
  for (const [path, heading] of [
    ["/templates", "Templates"],
    ["/auth", "Authentication"],
    ["/changes", "Changes"],
  ]) {
    await page.goto(path);
    await expect(page.getByRole("heading", { name: heading, exact: true })).toBeVisible();
  }
  const session = await page.evaluate(async () => {
    const session = await fetch("/api/v1/session");
    const csrf = session.headers.get("x-csrf-token")!;
    const rejected = await fetch("/api/v1/github-auth-profiles/test", {
      method: "PUT",
      body: "{}",
    });
    const validated = await fetch("/api/v1/github-auth-profiles/test", {
      method: "PUT",
      headers: { "x-csrf-token": csrf, "if-none-match": "*", "content-type": "application/json" },
      body: "{}",
    });
    return [rejected.status, validated.status];
  });
  expect(session).toEqual([403, 422]);
  await page.screenshot({ path: "test-results/oidc-desktop.png", fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/fleets");
  await expect(page.getByRole("heading", { name: "Fleets", exact: true })).toBeVisible();
  await page.screenshot({ path: "test-results/oidc-mobile.png", fullPage: true });
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.getByRole("button", { name: "Session menu" }).click();
  await page.getByRole("menuitem", { name: "Sign out", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Sign in to Shaula" })).toBeVisible();
  expect((await page.request.get("/api/v1/session")).status()).toBe(401);
  expect((await page.request.get(scripts[0])).status()).toBe(401);
});
