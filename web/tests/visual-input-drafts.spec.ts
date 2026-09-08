import { expect, test } from "@playwright/test";
import { mockApi } from "./fixtures";
import {
  acceptCreate,
  contract,
  field,
  loadTemplate,
  mockContract,
  openCreate,
} from "./visual-input-fixtures";

function imageContract(key = "kubernetes-linux", revision = 3, value = "old-image") {
  return contract(
    [field("runner_image", [JSON.stringify(value)], true, "Runner image")],
    key,
    revision,
  );
}

test("promotion never silently replaces the captured revision or retries a rejected submission", async ({
  page,
}) => {
  await mockApi(page);
  let active = 3;
  await page.route("**/api/v1/template-profiles/kubernetes-linux", (route) =>
    route.fulfill({
      json: {
        key: "kubernetes-linux",
        incarnation: "template-inc",
        desiredRevision: active,
        activeRevision: active,
        status: "Active",
      },
    }),
  );
  await mockContract(page, imageContract());
  await mockContract(page, imageContract("kubernetes-linux", 4, "new-image"));
  await openCreate(page);
  await loadTemplate(page);
  await page
    .getByRole("combobox", { name: "Runner image", exact: true })
    .selectOption({ index: 1 });
  active = 4;
  let writes = 0;
  await page.route("**/api/v1/fleets/visual-build", (route) => {
    writes += 1;
    expect(route.request().postDataJSON().template_profile_ref).toEqual({
      key: "kubernetes-linux",
      revision: 3,
    });
    expect(route.request().postDataJSON().template_inputs).toEqual({ runner_image: "old-image" });
    return route.fulfill({
      status: 409,
      json: {
        code: "TemplateNotActive",
        detail: "The selected template revision is no longer Active",
      },
    });
  });
  await page.getByRole("dialog").getByRole("button", { name: "Create fleet" }).click();
  await expect(page.getByRole("alert")).toContainText("no longer Active");
  expect(writes).toBe(1);
  await expect(page.getByRole("combobox", { name: "Runner image", exact: true })).not.toHaveValue(
    "",
  );
  await page.getByRole("button", { name: "Load latest Active", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Discard inputs and switch", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Cancel switch", exact: true }).click();
  await expect(page.getByRole("combobox", { name: "Runner image", exact: true })).not.toHaveValue(
    "",
  );
  await page.getByRole("button", { name: "Load latest Active", exact: true }).click();
  await page.getByRole("button", { name: "Discard inputs and switch", exact: true }).click();
  const image = page.getByRole("combobox", { name: "Runner image", exact: true });
  await expect(image).toHaveValue("");
  await expect(image.locator("option").filter({ hasText: "old-image" })).toHaveCount(0);
  await image.selectOption({ index: 1 });
  await acceptCreate(page, (body) => {
    expect(JSON.parse(body).template_profile_ref).toEqual({ key: "kubernetes-linux", revision: 4 });
    expect(JSON.parse(body).template_inputs).toEqual({ runner_image: "new-image" });
  });
  expect(writes).toBe(1);
});

test("failed template switches retain input drafts and cancelling returns to the original template", async ({
  page,
}) => {
  await mockApi(page);
  await mockContract(page, imageContract());
  await page.route("**/api/v1/template-profiles/broken-template", (route) =>
    route.fulfill({
      status: 503,
      json: { code: "Unavailable", detail: "Template registry temporarily unavailable" },
    }),
  );
  await openCreate(page);
  await loadTemplate(page);
  await page
    .getByRole("combobox", { name: "Runner image", exact: true })
    .selectOption({ index: 1 });
  await loadTemplate(page, "broken-template");
  await expect(page.getByRole("dialog")).toContainText("Template registry temporarily unavailable");
  await expect(page.getByRole("combobox", { name: "Runner image", exact: true })).not.toHaveValue(
    "",
  );
  await expect(
    page.getByRole("dialog").getByRole("button", { name: "Create fleet" }),
  ).toBeDisabled();
  await page.getByRole("button", { name: "Cancel switch", exact: true }).click();
  await expect(page.getByLabel("Template profile", { exact: true })).toHaveValue(
    "kubernetes-linux",
  );
  await acceptCreate(page, (body) =>
    expect(JSON.parse(body).template_inputs).toEqual({ runner_image: "old-image" }),
  );
});

test("late contract responses cannot overwrite a newer template selection", async ({ page }) => {
  await mockApi(page);
  let releaseSlow!: () => void;
  const slow = new Promise<void>((resolve) => {
    releaseSlow = resolve;
  });
  let requestedSlow!: () => void;
  const slowRequested = new Promise<void>((resolve) => {
    requestedSlow = resolve;
  });
  await page.route(
    "**/api/v1/template-profiles/slow-template/revisions/3/input-contract",
    async (route) => {
      requestedSlow();
      await slow;
      await route
        .fulfill({ json: imageContract("slow-template", 3, "slow-image") })
        .catch(() => {});
    },
  );
  await mockContract(page, imageContract("fast-template", 3, "fast-image"));
  await openCreate(page);
  await loadTemplate(page, "slow-template");
  await slowRequested;
  await loadTemplate(page, "fast-template");
  await expect(page.getByRole("combobox", { name: "Runner image", exact: true })).toBeVisible();
  await page
    .getByRole("combobox", { name: "Runner image", exact: true })
    .selectOption({ index: 1 });
  releaseSlow();
  await expect(page.getByLabel("Template profile", { exact: true })).toHaveValue("fast-template");
  await acceptCreate(page, (body) => {
    expect(JSON.parse(body).template_profile_ref).toEqual({ key: "fast-template", revision: 3 });
    expect(JSON.parse(body).template_inputs).toEqual({ runner_image: "fast-image" });
  });
});

test("new templates with no Active revision show a blocking reason and can be retried", async ({
  page,
}) => {
  await mockApi(page);
  let active: number | null = null;
  await page.route("**/api/v1/template-profiles/kubernetes-linux", (route) =>
    route.fulfill({
      json: {
        key: "kubernetes-linux",
        incarnation: "template-inc",
        desiredRevision: 3,
        activeRevision: active,
        status: active ? "Active" : "Validating",
      },
    }),
  );
  await openCreate(page);
  await loadTemplate(page);
  await expect(page.getByRole("dialog")).toContainText(/no Active revision/i);
  await expect(
    page.getByRole("dialog").getByRole("button", { name: "Create fleet" }),
  ).toBeDisabled();
  active = 3;
  await page.getByRole("button", { name: "Retry input contract", exact: true }).click();
  await expect(page.getByRole("dialog")).toContainText(
    "This template needs no input configuration.",
  );
  await acceptCreate(page, (body) => expect(JSON.parse(body).template_inputs).toEqual({}));
});
