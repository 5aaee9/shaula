import { expect, test } from "@playwright/test";
import { fleetForm, mockApi, profile } from "./fixtures";

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
  const form = fleetForm(page);
  await form.getByLabel("Fleet key").fill("pool-fleet");
  await form.getByLabel("Owner", { exact: true }).fill("acme");
  await form
    .getByLabel("GitHub authentication profile", { exact: true })
    .selectOption("github-build");
  await form.getByRole("radio", { name: "Weighted pool" }).click();
  await form.getByLabel("Template profile").selectOption("kubernetes-linux");
  await form.getByRole("spinbutton", { name: "Weight" }).fill("20");
  await expect(form).toContainText("independently by weight");
  await form.getByRole("button", { name: "Create fleet", exact: true }).click();
  await expect
    .poll(() => submitted)
    .toMatchObject({
      template_pool: {
        failure_policy: "backpressure",
        members: [{ key: "member-1", template_profile_ref: "kubernetes-linux", weight: 20 }],
      },
    });
});

test("removing a pool member keeps the remaining member's loaded template inputs", async ({
  page,
}) => {
  await mockApi(page);
  await page.route("**/api/v1/template-profiles", (route) =>
    route.fulfill({ json: { profiles: [profile("kubernetes-linux"), profile("kubernetes-gpu")] } }),
  );
  const contractReads: string[] = [];
  page.on("request", (request) => {
    if (request.url().endsWith("/input-contract")) contractReads.push(request.url());
  });
  await page.goto("/fleets");
  await page.getByRole("button", { name: "Create fleet" }).click();
  const form = fleetForm(page);
  await form.getByRole("radio", { name: "Weighted pool" }).click();
  await form.getByLabel("Template profile").selectOption("kubernetes-linux");
  await form.getByRole("button", { name: "Add member" }).click();
  await form.getByLabel("Template profile").nth(1).selectOption("kubernetes-gpu");
  await expect(form.getByText("This template needs no input configuration.")).toHaveCount(2);
  const readsBeforeRemove = contractReads.length;

  await form.getByRole("button", { name: "Remove member-1" }).click();
  await expect(form.getByLabel("Member key")).toHaveValue("member-2");
  await expect(form.getByLabel("Template profile")).toHaveValue("kubernetes-gpu");
  await expect(form.getByText("This template needs no input configuration.")).toBeVisible();
  // The surviving row keeps its identity, so its contract is not read again.
  await page.waitForTimeout(300);
  expect(contractReads.length).toBe(readsBeforeRemove);
});
