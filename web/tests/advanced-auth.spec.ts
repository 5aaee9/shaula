import { expect, test } from "@playwright/test";
import { mockApi } from "./fixtures";

test("additional authentication targets reveal invalid fields and retain scope when collapsed", async ({
  page,
}) => {
  await mockApi(page);
  await page.setViewportSize({ width: 390, height: 844 });
  let submitted: unknown = null;
  let writes = 0;
  await page.route("**/api/v1/github-auth-profiles/compact-auth", async (route) => {
    if (route.request().method() !== "PUT") return route.fallback();
    writes++;
    submitted = route.request().postDataJSON();
    return route.fulfill({
      status: 202,
      json: { changeId: "compact-auth-1", state: "Accepted", revision: 1 },
    });
  });
  await page.goto("/auth");
  await page.getByRole("button", { name: "Create profile" }).click();
  const dialog = page.getByRole("dialog");
  const advanced = dialog.getByRole("button", { name: "Advanced settings" });
  await expect(advanced).toHaveAttribute("aria-expanded", "false");
  await expect(dialog.getByRole("button", { name: "Add selector" })).toBeHidden();
  await dialog.getByLabel("Profile key", { exact: true }).fill("compact-auth");
  await dialog.getByLabel("App ID", { exact: true }).fill("4863460");
  await dialog.getByLabel("Private key (PEM)").fill("-----BEGIN TEST-----");
  await dialog.getByLabel("Owner", { exact: true }).fill("Indexyz");
  await page.screenshot({
    path: "test-results/auth-dialog-compact.png",
    fullPage: true,
    animations: "disabled",
  });
  expect(await dialog.evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
  await advanced.click();
  await dialog.getByRole("button", { name: "Add selector" }).click();
  await dialog.getByLabel("Selector type").last().selectOption("account_repositories");
  await dialog.getByLabel("Account type").selectOption("user");
  const additionalOwner = dialog.getByLabel("Owner", { exact: true }).last();
  await advanced.click();
  await expect(additionalOwner).toBeHidden();
  await dialog.getByRole("button", { name: "Create profile" }).click();
  await expect(advanced).toHaveAttribute("aria-expanded", "true");
  await expect(additionalOwner).toBeVisible();
  await expect(additionalOwner).toBeFocused();
  expect(writes).toBe(0);
  // Whitespace-only scope cannot be submitted from a collapsed section either.
  await additionalOwner.fill("   ");
  await advanced.click();
  await dialog.getByRole("button", { name: "Create profile" }).click();
  await expect(advanced).toHaveAttribute("aria-expanded", "true");
  await expect(additionalOwner).toBeVisible();
  await expect(additionalOwner).toBeFocused();
  expect(await additionalOwner.evaluate((node) => (node as HTMLInputElement).validity.valid)).toBe(
    false,
  );
  expect(writes).toBe(0);
  await additionalOwner.fill("5aaee9");
  await advanced.click();
  await expect(additionalOwner).toBeHidden();
  await expect(dialog.getByText("1 additional target: 5aaee9 (user repositories)")).toBeVisible();
  await dialog.getByRole("button", { name: "Create profile" }).click();
  await expect(dialog).toHaveCount(0);
  expect(writes).toBe(1);
  expect(submitted).toEqual({
    kind: "github_app",
    schema_version: 2,
    app_id: "4863460",
    private_key: "-----BEGIN TEST-----",
    target_policy: [
      { kind: "organization", owner: "Indexyz" },
      { kind: "account_repositories", account_kind: "user", owner: "5aaee9" },
    ],
  });
});
