import { expect, test } from "@playwright/test";
import { fleet, mockApi } from "./fixtures";
import {
  acceptCreate,
  contract,
  field,
  loadTemplate,
  mockContract,
  openCreate,
} from "./visual-input-fixtures";

test("approved values preserve JSON types and large integers without default selections", async ({
  page,
}) => {
  await mockApi(page);
  await mockContract(
    page,
    contract([
      field(
        "runner_image",
        ['"approved-image"'],
        true,
        "Runner image",
        "Choose an approved runner image.",
      ),
      field("array", ["[]", '[1,"two"]']),
      field("boolean", ["false", "true"]),
      field("empty", ['""']),
      field("empty_object", ["{}"]),
      field("large", ["9007199254740993"]),
      field("mixed", ["0", '"0"', "null"]),
      field("nested", ['{"limit":9007199254740993,"enabled":false}']),
      field("zero", ["0"]),
    ]),
  );
  await page.setViewportSize({ width: 390, height: 844 });
  await openCreate(page);
  await loadTemplate(page);
  const image = page.getByRole("combobox", { name: "Runner image", exact: true });
  await expect(image).toBeVisible();
  await expect(image).toHaveValue("");
  await expect(page.getByLabel("boolean", { exact: true })).toBeVisible();
  await expect(page.getByLabel("Template inputs (JSON)")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Advanced settings" })).toHaveAttribute(
    "aria-expanded",
    "false",
  );
  // Even a single approved value requires an explicit choice.
  await image.focus();
  await image.press("ArrowDown");
  await image.press("Tab");
  for (const key of ["array", "boolean", "empty", "empty_object", "large", "nested", "zero"])
    await page.getByLabel(key, { exact: true }).selectOption({ index: 1 });
  await page.getByLabel("mixed", { exact: true }).selectOption({ index: 3 });
  await expect(page.getByLabel("mixed", { exact: true }).locator("option")).toHaveCount(4);
  await expect(
    page
      .getByLabel("mixed", { exact: true })
      .getByRole("option", { name: "0 (number)", exact: true }),
  ).toHaveCount(1);
  await expect(
    page
      .getByLabel("mixed", { exact: true })
      .getByRole("option", { name: "0 (string)", exact: true }),
  ).toHaveCount(1);
  await page.screenshot({
    path: "test-results/visual-inputs-mobile.png",
    fullPage: true,
    animations: "disabled",
  });
  expect(
    await page.getByRole("dialog").evaluate((node) => node.scrollWidth <= node.clientWidth),
  ).toBe(true);
  const advanced = page.getByRole("button", { name: "Advanced settings" });
  await advanced.click();
  await advanced.click();
  await expect(advanced).toHaveAttribute("aria-expanded", "false");
  await expect(page.getByLabel("mixed", { exact: true })).toBeVisible();
  await page.getByLabel("mixed", { exact: true }).selectOption({ index: 2 });
  await page.getByLabel("mixed", { exact: true }).selectOption({ index: 3 });
  await acceptCreate(page, (body) => {
    const payload = JSON.parse(body);
    expect(payload.template_profile_ref).toEqual({ key: "kubernetes-linux", revision: 3 });
    expect(payload.template_inputs).toMatchObject({
      runner_image: "approved-image",
      array: [],
      boolean: false,
      empty: "",
      empty_object: {},
      mixed: null,
      zero: 0,
    });
    expect(body).toContain('"large":9007199254740993');
    expect(body).toContain('"limit":9007199254740993');
  });
  expect(await page.evaluate(() => [localStorage.length, sessionStorage.length])).toEqual([0, 0]);
});

test("optional inputs stay visible when Advanced settings is collapsed and can be explicitly unset", async ({
  page,
}) => {
  await mockApi(page);
  await mockContract(
    page,
    contract([
      field("runner_image", ['"approved-image"'], true, "Runner image"),
      field(
        "choice",
        ["false", "0", '""', "null", "[]", "{}"],
        false,
        "Optional choice",
        '<img src=x onerror="alert(1)">',
      ),
    ]),
  );
  await openCreate(page);
  await loadTemplate(page);
  await page
    .getByRole("combobox", { name: "Runner image", exact: true })
    .selectOption({ index: 1 });
  const choice = page.getByLabel("Optional choice", { exact: true });
  const advanced = page.getByRole("button", { name: "Advanced settings" });
  await expect(advanced).toHaveAttribute("aria-expanded", "false");
  await expect(choice).toBeVisible();
  await expect(choice).toHaveValue("");
  await expect(page.getByText('<img src=x onerror="alert(1)">', { exact: true })).toBeVisible();
  await expect(page.getByRole("dialog").locator("img")).toHaveCount(0);
  await choice.selectOption({ index: 1 });
  await advanced.click();
  await advanced.click();
  await expect(advanced).toHaveAttribute("aria-expanded", "false");
  await expect(choice).toBeVisible();
  await expect(choice.locator("option:checked")).toContainText("No");
  await choice.selectOption({ index: 0 });
  await acceptCreate(page, (body) =>
    expect(JSON.parse(body).template_inputs).toEqual({ runner_image: "approved-image" }),
  );
});

test("root presets submit one complete approved configuration", async ({ page }) => {
  await mockApi(page);
  const identity = contract();
  await page.route(
    "**/api/v1/template-profiles/kubernetes-linux/revisions/3/input-contract",
    (route) =>
      route.fulfill({
        json: {
          version: identity.version,
          profileKey: identity.profileKey,
          incarnation: identity.incarnation,
          revision: identity.revision,
          artifactDigest: identity.artifactDigest,
          mode: "presets",
          presets: [
            { valueJson: '{"cpu":"small","memory":"low"}' },
            { valueJson: '{"cpu":"large","memory":"high","id":9007199254740993}' },
          ],
        },
      }),
  );
  await openCreate(page);
  await loadTemplate(page);
  const preset = page.getByRole("combobox", { name: "Input configuration", exact: true });
  await expect(preset).toBeVisible();
  await expect(preset).toHaveValue("");
  await expect(page.getByLabel("cpu", { exact: true })).toHaveCount(0);
  await preset.selectOption({ index: 2 });
  await acceptCreate(page, (body) => {
    expect(JSON.parse(body).template_inputs).toMatchObject({ cpu: "large", memory: "high" });
    expect(body).toContain('"id":9007199254740993');
  });
});

test("object identity treats prototype and Unicode keys as data while distinguishing integer and float tokens", async ({
  page,
}) => {
  await mockApi(page);
  const approvedObject = '{"__proto__":{"allowed":true},"😀":2,"é":1}';
  await mockContract(
    page,
    contract([
      field("numeric", ["1.0"], true, "Numeric choice"),
      field("object", [approvedObject], true, "Object choice"),
    ]),
  );
  const current = fleet("linux-build");
  current.spec.template_inputs = JSON.parse(
    '{"numeric":1,"object":{"é":1,"😀":2,"__proto__":{"allowed":true}}}',
  );
  await page.route("**/api/v1/fleets/linux-build", (route) => {
    if (route.request().method() === "GET")
      return route.fulfill({ json: current, headers: { etag: '"original:2"' } });
    const raw = route.request().postData()!;
    expect(raw).toContain('"numeric":1.0');
    const object = route.request().postDataJSON().template_inputs.object;
    expect(Object.hasOwn(object, "__proto__")).toBe(true);
    expect(object["__proto__"]).toEqual({ allowed: true });
    expect(object["😀"]).toBe(2);
    expect(object["é"]).toBe(1);
    return route.fulfill({
      status: 202,
      json: { changeId: "typed-identity", state: "Accepted", revision: 3 },
    });
  });
  await page.goto("/fleets/linux-build");
  await page.getByRole("button", { name: "Edit fleet" }).click();
  const numeric = page.getByRole("combobox", { name: "Numeric choice", exact: true });
  await expect(numeric.locator("option:checked")).toContainText("no longer selectable");
  const object = page.getByRole("combobox", { name: "Object choice", exact: true });
  await expect(object.locator("option:checked")).not.toContainText("no longer selectable");
  await expect(object).not.toHaveValue("");
  await numeric.selectOption({ label: "1.0" });
  await page.getByRole("button", { name: "Save changes" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(await page.evaluate(() => Object.hasOwn(Object.prototype, "allowed"))).toBe(false);
});

test("malformed approved JSON cannot become a selectable value or an empty fallback", async ({
  page,
}) => {
  await mockApi(page);
  await mockContract(
    page,
    contract([field("invalid", ['{"nested":[1,]}'], true, "Invalid choice")]),
  );
  await openCreate(page);
  await loadTemplate(page);
  await expect(page.getByRole("dialog")).toContainText(
    /Invalid input value|Invalid input contract/,
  );
  await expect(page.getByRole("combobox", { name: "Invalid choice", exact: true })).toHaveCount(0);
  await expect(
    page.getByRole("dialog").getByRole("button", { name: "Create fleet" }),
  ).toBeDisabled();
});
