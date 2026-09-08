import { expect, test } from "@playwright/test";
import { mockApi, profile } from "./fixtures";
import { artifactDigest, contract, field, mockContract, openCreate } from "./visual-input-fixtures";

test("Ready automatically refreshes to Active and becomes selectable with its inputs", async ({
  page,
}) => {
  await page.clock.install();
  await mockApi(page);
  let active = false;
  const current = () => profile("local-docker", active ? 3 : null, active ? "Active" : "Ready");
  await page.route("**/api/v1/template-profiles", (route) =>
    route.fulfill({ json: { profiles: [current()] } }),
  );
  await page.route("**/api/v1/template-profiles/local-docker", (route) =>
    route.fulfill({ json: { ...current(), platform: "docker", bindings_present: true } }),
  );
  await page.route("**/api/v1/template-profiles/local-docker/revisions/3", (route) =>
    route.fulfill({
      json: { artifactDigest, engineRef: "terraform", state: current().status, reason: null },
    }),
  );
  await mockContract(page, contract([field("image", ['"runner:approved"'], true)], "local-docker"));
  await page.goto("/templates?key=local-docker");
  await expect(
    page.getByRole("status").filter({ hasText: "Waiting for automatic activation" }),
  ).toBeVisible();
  active = true;
  await page.clock.fastForward(10_001);
  await expect(page.getByText("Waiting for automatic activation.", { exact: false })).toHaveCount(
    0,
  );
  await expect(page.getByRole("row").filter({ hasText: "local-docker" })).toContainText("Active");

  await openCreate(page);
  const template = page.getByRole("combobox", { name: "Template profile", exact: true });
  await template.selectOption("local-docker");
  await expect(template).toHaveValue("local-docker");
  await expect(page.getByRole("combobox", { name: "image", exact: true })).toBeVisible();
  await expect(page.getByRole("combobox", { name: "image", exact: true })).toHaveValue("");
});

test("rejected revision shows the server reason while the previous Active remains visible", async ({
  page,
}) => {
  await mockApi(page);
  const rejected = { ...profile("local-docker", 2, "Rejected"), desiredRevision: 3 };
  await page.route("**/api/v1/template-profiles", (route) =>
    route.fulfill({ json: { profiles: [rejected] } }),
  );
  await page.route("**/api/v1/template-profiles/local-docker", (route) =>
    route.fulfill({ json: rejected }),
  );
  await page.route("**/api/v1/template-profiles/local-docker/revisions/3", (route) =>
    route.fulfill({
      json: {
        artifactDigest,
        engineRef: "terraform",
        state: "Rejected",
        reason: "DependencyLockInvalid",
      },
    }),
  );
  await page.goto("/templates?key=local-docker");
  await expect(page.getByText("DependencyLockInvalid", { exact: true })).toBeVisible();
  const row = page.getByRole("row").filter({ hasText: "local-docker" });
  await expect(row).toContainText("r2");
  await expect(row).toContainText("r3");
  await expect(page.getByText("Waiting for automatic activation.", { exact: false })).toHaveCount(
    0,
  );
});
