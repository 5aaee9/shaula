import { expect, test, type Page } from "@playwright/test";
import { scopes } from "./fixtures";
import { dockerDigest, variables } from "./template-library-fixtures";
import {
  availableUpdate,
  currentDigest,
  mockTemplateUpdate,
  updateApi,
  updatePath,
} from "./template-update-fixtures";

function acceptUpdates(page: Page, bodies: unknown[]) {
  return page.route(updateApi, (route) => {
    expect(route.request().headers()["if-match"]).toBe('"template-inc:3"');
    expect(route.request().headers()["idempotency-key"]).toBeTruthy();
    bodies.push(route.request().postDataJSON());
    return route.fulfill({
      status: 202,
      json: { changeId: "one-click", state: "Accepted", revision: 4 },
    });
  });
}

test("an available default update is applied from the list without choosing a template", async ({
  page,
}, testInfo) => {
  await mockTemplateUpdate(page, { sourceKey: "docker", sourceUpdate: availableUpdate });
  const bodies: unknown[] = [];
  await acceptUpdates(page, bodies);
  await page.goto("/templates");
  const chip = page
    .getByRole("row", { name: /custom-docker/ })
    .getByRole("button", { name: "Update custom-docker from default" });
  await expect(chip).toHaveText("Update");
  await page.getByRole("table").screenshot({ path: testInfo.outputPath("update-chip-list.png") });
  await chip.click();
  const dialog = page.getByRole("dialog", { name: "Update custom-docker?" });
  await expect(dialog).toContainText("docker");
  await expect(dialog).toContainText(`sha256:${"a".repeat(12)}`);
  await expect(dialog).toContainText(`sha256:${"d".repeat(12)}`);
  await expect(dialog.getByRole("combobox")).toHaveCount(0);
  await page.screenshot({ path: testInfo.outputPath("quick-update-desktop.png") });
  await page.setViewportSize({ width: 390, height: 844 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(
    true,
  );
  await page.screenshot({ path: testInfo.outputPath("quick-update-mobile.png") });
  expect(bodies).toHaveLength(0);
  await dialog.getByRole("button", { name: "Update", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByText("Change for custom-docker")).toBeVisible();
  // Bindings and policy are inherited server-side, never resent.
  expect(bodies).toEqual([
    { artifact_digest: dockerDigest, engine_ref: "terraform", source_key: "docker" },
  ]);
});

test("up-to-date and unavailable sources offer no one-click update", async ({ page }) => {
  await mockTemplateUpdate(page, {
    sourceKey: "docker",
    digest: dockerDigest,
    sourceUpdate: { ...availableUpdate, state: "UpToDate" },
  });
  await page.goto("/templates");
  const row = page.getByRole("row", { name: /custom-docker/ });
  await expect(row).toContainText("docker");
  await expect(page.getByRole("button", { name: "Update custom-docker from default" })).toHaveCount(
    0,
  );

  await page.route("**/api/v1/template-profiles", (route) =>
    route.fulfill({
      json: {
        profiles: [
          {
            key: "custom-docker",
            incarnation: "template-inc",
            desiredRevision: 3,
            activeRevision: 3,
            status: "Active",
            sourceUpdate: {
              sourceKey: "docker",
              state: "Unavailable",
              targetDigest: null,
              targetEngine: null,
            },
          },
        ],
      },
    }),
  );
  await page.reload();
  await expect(page.getByRole("row", { name: /custom-docker/ })).toContainText(
    "Source unavailable",
  );
  await expect(page.getByRole("button", { name: "Update custom-docker from default" })).toHaveCount(
    0,
  );
});

test("a new required binding sends the one-click update to review", async ({ page }) => {
  await mockTemplateUpdate(page, { sourceKey: "docker", sourceUpdate: availableUpdate });
  await page.route("**/api/v1/template-artifacts/*/variables", (route) =>
    route.fulfill({
      json: {
        ...variables,
        bindings: [
          ...variables.bindings,
          {
            key: "region",
            label: "Region",
            typeName: "string",
            required: true,
            sensitive: false,
            options: [],
          },
        ],
      },
    }),
  );
  let writes = 0;
  await page.route(updateApi, (route) => {
    writes++;
    return route.abort();
  });
  await page.goto("/templates");
  await page.getByRole("button", { name: "Update custom-docker from default" }).click();
  const dialog = page.getByRole("dialog", { name: "Update custom-docker?" });
  await expect(dialog.getByRole("status")).toContainText("Region");
  await expect(dialog.getByRole("button", { name: "Update", exact: true })).toBeDisabled();
  await dialog.getByRole("button", { name: "Review changes" }).click();
  await expect(page).toHaveURL(new RegExp(`${updatePath}$`));
  await expect(page.getByRole("region", { name: "Revision changes" })).toContainText(dockerDigest);
  expect(writes).toBe(0);
});

test("a rejected one-click update keeps the dialog open with a review path", async ({ page }) => {
  await mockTemplateUpdate(page, { sourceKey: "docker", sourceUpdate: availableUpdate });
  let writes = 0;
  await page.route(updateApi, (route) => {
    writes++;
    return route.fulfill({
      status: 422,
      json: { code: "Validation", detail: "The selected default template has changed." },
    });
  });
  await page.goto("/templates");
  await page.getByRole("button", { name: "Update custom-docker from default" }).click();
  const dialog = page.getByRole("dialog", { name: "Update custom-docker?" });
  await dialog.getByRole("button", { name: "Update", exact: true }).click();
  await expect(dialog.getByRole("alert")).toContainText("default template has changed");
  await expect(dialog.getByRole("button", { name: "Update", exact: true })).toBeDisabled();
  await expect(dialog.getByRole("button", { name: "Review changes" })).toBeEnabled();
  expect(writes).toBe(1);
});

test("the detail page updates from its header and returns to templates", async ({ page }) => {
  await mockTemplateUpdate(page, { sourceKey: "docker", sourceUpdate: availableUpdate });
  const bodies: unknown[] = [];
  await acceptUpdates(page, bodies);
  await page.goto("/templates/custom-docker");
  const header = page.locator("[data-slot=page-header]");
  // Settings edit in place only while the source is current.
  await expect(header.getByRole("button", { name: "Edit settings" })).toHaveCount(0);
  await expect(header).toContainText("docker");
  await header.getByRole("button", { name: "Update custom-docker from default" }).click();
  await page
    .getByRole("dialog", { name: "Update custom-docker?" })
    .getByRole("button", { name: "Update", exact: true })
    .click();
  await expect(page).toHaveURL(/\/templates\?key=custom-docker$/);
  await expect(page.getByText("Change for custom-docker")).toBeVisible();
  expect(bodies).toHaveLength(1);
});

test("the update chip is disabled without publish permission", async ({ page }) => {
  await mockTemplateUpdate(page, {
    sourceKey: "docker",
    sourceUpdate: availableUpdate,
    permissions: scopes.filter((scope) => scope !== "template.publish"),
  });
  await page.goto("/templates");
  await expect(
    page.getByRole("button", { name: "Update custom-docker from default" }),
  ).toBeDisabled();
  await page.goto("/templates/custom-docker");
  await expect(
    page.getByRole("button", { name: "Update custom-docker from default" }),
  ).toBeDisabled();
});

test("a current source edits settings in place without a code comparison", async ({
  page,
}, testInfo) => {
  await mockTemplateUpdate(page, {
    sourceKey: "docker",
    digest: dockerDigest,
    sourceUpdate: { ...availableUpdate, state: "UpToDate" },
  });
  let written: Record<string, unknown> | null = null;
  await page.route(updateApi, (route) => {
    written = route.request().postDataJSON();
    return route.fulfill({
      status: 202,
      json: { changeId: "settings", state: "Accepted", revision: 4 },
    });
  });
  await page.goto("/templates/custom-docker");
  const header = page.locator("[data-slot=page-header]");
  await expect(
    header.getByRole("button", { name: "Update custom-docker from default" }),
  ).toHaveCount(0);
  await expect(header.getByLabel("Default template")).toBeVisible();
  // One revision fact while Active and Desired agree; the source is not repeated below.
  await expect(page.getByText("Active revision")).toHaveCount(0);
  await expect(page.getByText("Default source")).toHaveCount(0);
  await page.screenshot({
    path: testInfo.outputPath("template-detail-current.png"),
    fullPage: true,
  });
  await header.getByRole("button", { name: "Edit settings" }).click();
  await expect(page).toHaveURL(new RegExp(`${updatePath}$`));
  await expect(page.getByRole("heading", { name: "Edit custom-docker settings" })).toBeVisible();
  await expect(page.getByRole("region", { name: "Revision changes" })).toHaveCount(0);
  const save = page.getByRole("button", { name: "Save", exact: true });
  await expect(save).toBeDisabled();
  await page.getByRole("button", { name: "Use declared options" }).click();
  await save.click();
  await expect(page).toHaveURL(/\/templates\?key=custom-docker$/);
  expect(written).toMatchObject({
    artifact_digest: dockerDigest,
    engine_ref: "terraform",
    source_key: "docker",
  });
  expect(written!.fleet_input_policy).toBeTruthy();
});

test("a custom template only offers a new revision", async ({ page }) => {
  await mockTemplateUpdate(page);
  await page.goto("/templates/custom-docker");
  const header = page.locator("[data-slot=page-header]");
  await expect(header.getByRole("button", { name: "New revision" })).toBeEnabled();
  await expect(header.getByRole("button", { name: "Edit settings" })).toHaveCount(0);
  await expect(header.getByRole("button", { name: /from default/ })).toHaveCount(0);
});

test("a new revision keeps the recorded default source selected", async ({ page }) => {
  await mockTemplateUpdate(page, { sourceKey: "docker", sourceUpdate: availableUpdate });
  await page.route("**/api/v1/template-profiles/custom-docker", (route) => {
    if (route.request().method() === "GET") return route.fallback();
    expect(route.request().postDataJSON()).toMatchObject({
      artifact_digest: dockerDigest,
      engine_ref: "terraform",
      source_key: "docker",
    });
    return route.fulfill({
      status: 202,
      json: { changeId: "kept-source", state: "Accepted", revision: 4 },
    });
  });
  await page.goto("/templates/custom-docker/revisions/new");
  await expect(page.getByRole("radio", { name: "Default template", exact: true })).toBeChecked();
  await expect(page.getByRole("combobox", { name: "Default template", exact: true })).toHaveValue(
    "docker",
  );
  await page.getByRole("button", { name: "Publish", exact: true }).click();
  await expect(page).toHaveURL(/\/templates\?key=custom-docker$/);
});

test("a revision without a recorded source still starts a new revision from an archive", async ({
  page,
}) => {
  await mockTemplateUpdate(page, { digest: currentDigest });
  await page.goto("/templates/custom-docker/revisions/new");
  await expect(page.getByRole("radio", { name: "Upload archive", exact: true })).toBeChecked();
});
