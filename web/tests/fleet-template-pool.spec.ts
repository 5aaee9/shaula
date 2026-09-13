import { expect, test } from "@playwright/test";
import { mockApi } from "./fixtures";

test("creates a weighted template pool with independent random draw semantics", async ({
  page,
}) => {
  await mockApi(page);
  let submitted: any;
  await page.route("**/api/v1/fleets/pool-fleet", async (route) => {
    if (route.request().method() === "PUT") {
      submitted = JSON.parse(route.request().postData() || "{}");
      return route.fulfill({ json: { changeId: "change-1", state: "Accepted", revision: 1 } });
    }
    return route.continue();
  });
  await page.goto("/fleets");
  await page.getByRole("button", { name: "Create fleet" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Fleet key").fill("pool-fleet");
  await dialog.getByLabel("Owner", { exact: true }).fill("acme");
  await dialog
    .getByLabel("GitHub authentication profile", { exact: true })
    .selectOption("github-build");
  await dialog.getByRole("radio", { name: "Weighted pool" }).click();
  await dialog.getByLabel("Template profile").selectOption("kubernetes-linux");
  await dialog.getByRole("spinbutton", { name: "Weight" }).fill("20");
  await expect(dialog).toContainText("independently by weight");
  await dialog.getByRole("button", { name: "Create fleet", exact: true }).click();
  await expect
    .poll(() => submitted)
    .toMatchObject({
      template_pool: {
        failure_policy: "backpressure",
        members: [{ key: "member-1", template_profile_ref: "kubernetes-linux", weight: 20 }],
      },
    });
});
