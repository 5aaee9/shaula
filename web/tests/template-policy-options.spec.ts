import { expect, test, type Page } from "@playwright/test";
import type { TemplateVariable } from "../src/lib/template-variables";
import { dockerDigest, mockTemplateLibrary } from "./template-library-fixtures";
import { mockTemplateUpdate, updateApi, updatePath } from "./template-update-fixtures";

function parameter(
  key: string,
  label: string,
  options: string[],
  defaultValueJson?: string,
): TemplateVariable {
  return {
    key,
    label,
    typeName: "any",
    required: false,
    sensitive: false,
    defaultValueJson,
    options: options.map((valueJson) => ({ valueJson })),
  };
}

async function openPolicyForm(page: Page, parameters: TemplateVariable[]) {
  await mockTemplateLibrary(page);
  await page.route("**/api/v1/template-artifacts/*/variables", (route) =>
    route.fulfill({
      json: { artifactDigest: dockerDigest, available: true, bindings: [], parameters },
    }),
  );
  await page.goto("/templates");
  await page.getByRole("button", { name: "Use template docker" }).click();
  await expect(page.getByRole("region", { name: "Fleet input variables" })).toBeVisible();
}

async function policyEditor(page: Page) {
  await page.getByRole("button", { name: "Advanced settings" }).click();
  return page.getByLabel("Fleet input policy (JSON)");
}

async function publish(page: Page, key: string) {
  let body = "";
  await page.route(`**/api/v1/template-profiles/${key}`, (route) => {
    if (route.request().method() === "GET") return route.fallback();
    body = route.request().postData()!;
    expect(route.request().method()).toBe("PUT");
    return route.fulfill({
      status: 202,
      json: { changeId: "policy-options-published", state: "Accepted", revision: 1 },
    });
  });
  await page.getByLabel("Profile key").fill(key);
  await page.getByRole("button", { name: "Publish", exact: true }).click();
  await expect(page).toHaveURL(new RegExp(`/templates\\?key=${key}$`));
  return body;
}

test("declared approvals are independent checkboxes with no automatic default or sole selection", async ({
  page,
}) => {
  await openPolicyForm(page, [
    parameter("cpu_request", "CPU request", ['"500m"', '"1"', '"2"'], '"500m"'),
    parameter("runner_image", "Runner image", ['"runner:official"'], '"runner:official"'),
  ]);
  const cpu = page.getByRole("group", { name: "CPU request", exact: true });
  const small = cpu.getByRole("checkbox", { name: "500m (string)", exact: true });
  const medium = cpu.getByRole("checkbox", { name: "1 (string)", exact: true });
  const large = cpu.getByRole("checkbox", { name: "2 (string)", exact: true });
  const image = page
    .getByRole("group", { name: "Runner image", exact: true })
    .getByRole("checkbox", { name: "runner:official (string)", exact: true });
  for (const checkbox of [small, medium, large, image]) await expect(checkbox).not.toBeChecked();
  const policy = await policyEditor(page);
  await expect(policy).toHaveValue("{}");

  await small.check();
  await medium.focus();
  await page.keyboard.press("Space");
  await expect(medium).toBeChecked();
  await expect(small).toBeChecked();
  await small.uncheck();
  await image.check();
  await expect(large).not.toBeChecked();
  await expect(policy).toHaveValue('{"cpu_request":["1"],"runner_image":["runner:official"]}');
  const body = await publish(page, "selected-policy");
  expect(JSON.parse(body).fleet_input_policy).toEqual({
    cpu_request: ["1"],
    runner_image: ["runner:official"],
  });
});

test("individual approval changes preserve manual choices, unrelated keys and raw numbers", async ({
  page,
}) => {
  await openPolicyForm(page, [parameter("cpu_request", "CPU request", ['"500m"', '"1"', '"2"'])]);
  const policy = await policyEditor(page);
  await policy.fill(
    '{"cpu_request":["manual-large","500m"],"quota":[9007199254740993],"__proto__":[false]}',
  );
  const cpu = page.getByRole("group", { name: "CPU request", exact: true });
  const small = cpu.getByRole("checkbox", { name: "500m (string)", exact: true });
  const medium = cpu.getByRole("checkbox", { name: "1 (string)", exact: true });
  await expect(small).toBeChecked();
  await expect(medium).not.toBeChecked();
  await medium.check();
  await small.uncheck();
  const expected =
    '{"cpu_request":["manual-large","1"],"quota":[9007199254740993],"__proto__":[false]}';
  await expect(policy).toHaveValue(expected);
  const body = await publish(page, "preserved-policy");
  expect(body).toContain(`"fleet_input_policy":${expected}`);
});

test("checkbox approvals keep false, zero, empty string, null and large integer distinct", async ({
  page,
}) => {
  await openPolicyForm(page, [
    parameter("choice", "Typed choices", ["false", "0", '""', "null", "9007199254740993"]),
  ]);
  const choices = page.getByRole("group", { name: "Typed choices", exact: true });
  for (const name of [
    "No (boolean)",
    "0 (number)",
    "Empty string (string)",
    "Null (null)",
    "9007199254740993 (number)",
  ]) {
    const checkbox = choices.getByRole("checkbox", { name, exact: true });
    await expect(checkbox).not.toBeChecked();
    await checkbox.check();
  }
  await choices.getByRole("checkbox", { name: "0 (number)", exact: true }).uncheck();
  await expect(choices.getByRole("checkbox", { name: "No (boolean)", exact: true })).toBeChecked();
  const expected = '{"choice":[false,"",null,9007199254740993]}';
  const policy = await policyEditor(page);
  await expect(policy).toHaveValue(expected);
  const body = await publish(page, "typed-policy");
  expect(body).toContain(`"fleet_input_policy":${expected}`);
});

test("removing the last approved option keeps an explicit empty approval set", async ({ page }) => {
  await openPolicyForm(page, [parameter("image", "Runner image", ['"runner:official"'])]);
  const checkbox = page
    .getByRole("group", { name: "Runner image", exact: true })
    .getByRole("checkbox", { name: "runner:official (string)", exact: true });
  await checkbox.check();
  await checkbox.uncheck();
  const policy = await policyEditor(page);
  await expect(policy).toHaveValue('{"image":[]}');
  await page.getByRole("button", { name: "Use declared options" }).click();
  await expect(checkbox).not.toBeChecked();
  await expect(policy).toHaveValue('{"image":[]}');
});

test("invalid policy disables visual approval and preserves the manual draft", async ({ page }) => {
  await openPolicyForm(page, [parameter("image", "Runner image", ['"runner:official"'])]);
  const policy = await policyEditor(page);
  const checkbox = page
    .getByRole("group", { name: "Runner image", exact: true })
    .getByRole("checkbox", { name: "runner:official (string)", exact: true });
  for (const invalid of ['{"image":', "[]", '{"image":"manual-scalar"}']) {
    await policy.fill(invalid);
    await expect(checkbox).toBeDisabled();
    await expect(policy).toHaveValue(invalid);
  }
  await policy.fill('{"image":["runner:official"]}');
  await expect(checkbox).toBeEnabled();
  await expect(checkbox).toBeChecked();
  await checkbox.uncheck();
  await expect(policy).toHaveValue('{"image":[]}');
});

test("optional scalar parameters accept an explicit custom approval without adopting defaults", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 1200 });
  await openPolicyForm(page, [
    parameter("cpu_request", "CPU request", ['"500m"', '"1"', '"2"'], '"500m"'),
    {
      ...parameter("cpu_limit", "CPU limit", [], '"2"'),
      typeName: "string",
    },
  ]);
  const field = page.getByRole("group", { name: "CPU limit", exact: true });
  const input = field.getByLabel("Add approved value for CPU limit", { exact: true });
  await expect(input).toHaveValue("");
  await expect(input).toHaveAttribute("placeholder", "2");
  const policy = await policyEditor(page);
  await expect(policy).toHaveValue("{}");
  await input.fill("4");
  await field.getByRole("button", { name: "Add value", exact: true }).click();
  await expect(field.getByRole("checkbox", { name: "4 (string)", exact: true })).toBeChecked();
  await expect(policy).toHaveValue('{"cpu_limit":["4"]}');
  await page.getByRole("button", { name: "Advanced settings" }).click();
  const form = page.getByRole("form", { name: "Template configuration" });
  expect(await form.evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(
    true,
  );
  await page.getByRole("region", { name: "Fleet input variables" }).screenshot({
    path: "test-results/template-policy-options-mobile.png",
    animations: "disabled",
  });
  const body = await publish(page, "optional-custom-policy");
  expect(JSON.parse(body).fleet_input_policy).toEqual({ cpu_limit: ["4"] });
});

test("updating from declared options lets the publisher narrow the replacement policy", async ({
  page,
}) => {
  await mockTemplateUpdate(page, { sourceKey: "docker" });
  let body = "";
  await page.route(updateApi, (route) => {
    expect(route.request().method()).toBe("POST");
    body = route.request().postData()!;
    return route.fulfill({
      status: 202,
      json: { changeId: "policy-subset-updated", state: "Accepted", revision: 4 },
    });
  });
  await page.goto(updatePath);
  await page.getByRole("button", { name: "Use declared options" }).click();
  const image = page.getByRole("group", { name: "Runner image", exact: true });
  await expect(image.getByRole("checkbox", { name: "runner:stable (string)" })).toBeChecked();
  await image.getByRole("checkbox", { name: "runner:canary (string)" }).uncheck();
  await page.getByRole("button", { name: "Update", exact: true }).click();
  await expect(page).toHaveURL(/\/templates\?key=custom-docker$/);
  expect(body).toContain(
    '"fleet_input_policy":{"runner_image":["runner:stable"],"quota":[9007199254740993]}',
  );
  expect(body).not.toContain("never-display-this");
});

test("optional custom scalars preserve exact numbers, false and empty strings with explicit defaults", async ({
  page,
}) => {
  await openPolicyForm(page, [
    { ...parameter("quota", "Quota", [], "9007199254740995"), typeName: "number" },
    { ...parameter("enabled", "Enabled", [], "true"), typeName: "boolean" },
    { ...parameter("name", "Optional name", [], '"default-name"'), typeName: "string" },
  ]);
  await page.getByLabel("Profile key").fill("custom-scalar-policy");
  let writes = 0;
  page.on("request", (request) => {
    if (request.method() === "PUT" && request.url().endsWith("/custom-scalar-policy")) writes++;
  });
  const policy = await policyEditor(page);
  const quota = page.getByRole("group", { name: "Quota", exact: true });
  const number = quota.getByLabel("Add approved value for Quota", { exact: true });
  const enabled = page.getByRole("group", { name: "Enabled", exact: true });
  const empty = page.getByRole("group", { name: "Optional name", exact: true });
  await expect(number).toHaveValue("");
  await expect(number).toHaveAttribute("placeholder", "9007199254740995");
  await expect(enabled.getByRole("radio", { name: "Yes", exact: true })).not.toBeChecked();
  await expect(enabled.getByRole("radio", { name: "No", exact: true })).not.toBeChecked();
  await expect(enabled.getByRole("button", { name: "Add value", exact: true })).toBeDisabled();
  await expect(policy).toHaveValue("{}");

  await number.fill("9007199254740993");
  await number.press("Enter");
  await expect(quota.getByRole("checkbox", { name: "9007199254740993 (number)" })).toBeChecked();
  await expect(policy).toHaveValue('{"quota":[9007199254740993]}');
  expect(writes).toBe(0);
  await quota.getByRole("button", { name: "Use default value", exact: true }).click();
  await expect(policy).toHaveValue('{"quota":[9007199254740993,9007199254740995]}');
  await enabled.getByRole("radio", { name: "No", exact: true }).check();
  await expect(policy).toHaveValue('{"quota":[9007199254740993,9007199254740995]}');
  await enabled.getByRole("button", { name: "Add value", exact: true }).click();
  await expect(enabled.getByRole("checkbox", { name: "No (boolean)" })).toBeChecked();
  await expect(empty.getByLabel("Add approved value for Optional name")).toHaveValue("");
  await empty.getByRole("button", { name: "Add value", exact: true }).click();
  await expect(empty.getByRole("checkbox", { name: "Empty string (string)" })).toBeChecked();
  const expected = '{"quota":[9007199254740993,9007199254740995],"enabled":[false],"name":[""]}';
  await expect(policy).toHaveValue(expected);
  expect(writes).toBe(0);
  const body = await publish(page, "custom-scalar-policy");
  expect(body).toContain(`"fleet_input_policy":${expected}`);
  expect(writes).toBe(1);
});

test("optional bindings show defaults but write only an explicit edit or default selection", async ({
  page,
}) => {
  await mockTemplateLibrary(page);
  await page.goto("/templates");
  await page.getByRole("button", { name: "Use template docker" }).click();
  const host = page.getByLabel("Docker host", { exact: true });
  await expect(host).toHaveValue("");
  await expect(host).toHaveAttribute("placeholder", "unix:///var/run/docker.sock");
  const policy = await policyEditor(page);
  const bindings = page.getByLabel("Bindings (JSON)");
  await expect(bindings).toHaveValue("{}");
  await host.fill("tcp://custom:2375");
  await expect(bindings).toHaveValue('{"docker_host":"tcp://custom:2375"}');
  await page.getByRole("button", { name: "Clear Docker host", exact: true }).click();
  await expect(host).toHaveValue("");
  await expect(bindings).toHaveValue("{}");
  await page.getByRole("button", { name: "Use default for Docker host", exact: true }).click();
  await expect(host).toHaveValue("unix:///var/run/docker.sock");
  await expect(bindings).toHaveValue('{"docker_host":"unix:///var/run/docker.sock"}');
  await expect(page.getByLabel("Quota", { exact: true })).toHaveValue("");
  await expect(policy).toHaveValue("{}");
  const body = await publish(page, "optional-binding-default");
  expect(JSON.parse(body).bindings).toEqual({ docker_host: "unix:///var/run/docker.sock" });
  expect(JSON.parse(body).fleet_input_policy).toEqual({});
});
