import { expect, test } from "@playwright/test";

for (const [path, title] of [
  ["/auth/new", "Create authentication profile"],
  ["/auth/browser-missing/targets/edit", "Edit target policy for browser-missing"],
  ["/auth/browser-missing/rotate", "Rotate credential for browser-missing"],
]) {
  test(`OIDC returns to ${path} and supports a full document reload`, async ({ page, request }) => {
    await request.post("/__test/provider", { data: {} });
    const anonymous = await request.get(path, { maxRedirects: 0 });
    expect(anonymous.status()).toBe(302);
    expect(await anonymous.text()).not.toContain("<html");
    const destination = new URL(anonymous.headers().location, "https://localhost:5181");
    expect(destination.pathname).toBe("/auth/oidc/login");
    expect(destination.searchParams.get("return_to")).toBe(path);
    await page.goto(path);
    await expect(page).toHaveURL(`https://localhost:5181${path}`);
    await expect(page.getByRole("heading", { name: title, exact: true })).toBeVisible();
    const reloaded = await page.reload();
    expect(reloaded?.status()).toBe(200);
    await expect(page.getByRole("heading", { name: title, exact: true })).toBeVisible();
    if (path === "/auth/new") {
      await expect(page.getByRole("form", { name: "Authentication configuration" })).toBeVisible();
    } else {
      // No GitHub installation or credential is needed to verify protected deep links.
      await expect(page.getByRole("alert")).toBeVisible();
      await expect(page.getByRole("form", { name: "Authentication configuration" })).toHaveCount(0);
    }
  });
}
