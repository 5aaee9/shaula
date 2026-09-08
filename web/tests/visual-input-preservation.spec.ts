import { expect, test } from "@playwright/test";
import { fleet, mockApi, scopes } from "./fixtures";
import { contract, field, mockContract, openCreate } from "./visual-input-fixtures";

for (const failure of ["missing permission", "contract unavailable"] as const) {
  test(`${failure} locks template inputs while preserving capacity edits and the original bare reference`, async ({
    page,
  }) => {
    await mockApi(
      page,
      failure === "missing permission"
        ? scopes.filter((scope) => scope !== "template.read")
        : scopes,
    );
    const current = fleet("linux-build");
    const body = JSON.stringify({
      ...current,
      spec: {
        ...current.spec,
        template_profile_ref: "kubernetes-linux",
        template_inputs: { limit: "BIG_INTEGER", enabled: false },
      },
    }).replace('"BIG_INTEGER"', "9007199254740993");
    let writes = 0;
    let contractReads = 0;
    await page.route("**/api/v1/template-profiles/**/input-contract", (route) => {
      contractReads += 1;
      return route.fulfill({
        status: 503,
        json: { code: "Unavailable", detail: "Template artifact could not be read" },
      });
    });
    await page.route("**/api/v1/fleets/linux-build", (route) => {
      if (route.request().method() === "GET")
        return route.fulfill({
          contentType: "application/json",
          headers: { etag: '"original:2"' },
          body,
        });
      writes += 1;
      const raw = route.request().postData()!;
      expect(route.request().headers()["if-match"]).toBe('"original:2"');
      expect(JSON.parse(raw).template_profile_ref).toBe("kubernetes-linux");
      expect(JSON.parse(raw).template_inputs.enabled).toBe(false);
      expect(raw).toContain('"limit":9007199254740993');
      expect(JSON.parse(raw).capacity.max_runners).toBe(12);
      return route.fulfill({
        status: 202,
        json: { changeId: "capacity-only", state: "Accepted", revision: 3 },
      });
    });
    await page.goto("/fleets/linux-build");
    await page.getByRole("button", { name: "Edit fleet" }).click();
    await expect(page.getByRole("dialog")).toContainText(
      failure === "missing permission"
        ? /template.read|Template read permission/
        : "Template artifact could not be read",
    );
    await expect(page.getByLabel("Template profile", { exact: true })).toBeDisabled();
    await page.getByLabel("Maximum runners", { exact: true }).fill("12");
    await page.getByRole("button", { name: "Save changes" }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    expect(writes).toBe(1);
    if (failure === "missing permission") expect(contractReads).toBe(0);
    else expect(contractReads).toBeGreaterThan(0);
  });
}

test("new fleets cannot bypass template.read with empty inputs", async ({ page }) => {
  await mockApi(
    page,
    scopes.filter((scope) => scope !== "template.read"),
  );
  let writes = 0;
  await page.route("**/api/v1/fleets/visual-build", (route) => {
    writes += 1;
    return route.fulfill({ status: 500 });
  });
  await openCreate(page);
  await expect(page.getByRole("dialog")).toContainText(/template.read|Template read permission/);
  await expect(
    page.getByRole("dialog").getByRole("button", { name: "Create fleet" }),
  ).toBeDisabled();
  expect(writes).toBe(0);
});

test("unrecognized existing fields stay visible and preserve values for unrelated edits", async ({
  page,
}) => {
  await mockApi(page);
  await mockContract(
    page,
    contract([field("runner_image", ['"current-image"'], true, "Runner image")]),
  );
  const current = fleet("linux-build");
  current.spec.template_inputs = { runner_image: "old-image", obsolete: { limit: "BIG_INTEGER" } };
  const body = JSON.stringify(current).replace('"BIG_INTEGER"', "9007199254740993");
  let writes = 0;
  await page.route("**/api/v1/fleets/linux-build", (route) => {
    if (route.request().method() === "GET")
      return route.fulfill({
        contentType: "application/json",
        body,
        headers: { etag: '"original:2"' },
      });
    writes += 1;
    expect(route.request().postData()).toContain('"limit":9007199254740993');
    expect(route.request().postDataJSON().template_inputs.runner_image).toBe("old-image");
    expect(route.request().postDataJSON().capacity.max_runners).toBe(15);
    return route.fulfill({
      status: 202,
      json: { changeId: "preserved", state: "Accepted", revision: 3 },
    });
  });
  await page.goto("/fleets/linux-build");
  await page.getByRole("button", { name: "Edit fleet" }).click();
  await expect(page.getByRole("dialog")).toContainText("no longer selectable");
  await expect(page.getByRole("button", { name: "Remove obsolete", exact: true })).toBeVisible();
  await expect(page.getByRole("dialog")).toContainText("9007199254740993");
  await page.getByLabel("Maximum runners", { exact: true }).fill("15");
  await page.getByRole("button", { name: "Save changes" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(writes).toBe(1);
});

test("an operator explicitly replaces unsupported values and removes orphan fields", async ({
  page,
}) => {
  await mockApi(page);
  await mockContract(
    page,
    contract([field("runner_image", ['"current-image"'], true, "Runner image")]),
  );
  const current = fleet("linux-build");
  current.spec.template_inputs = { runner_image: "old-image", obsolete: ["old"] };
  let writes = 0;
  await page.route("**/api/v1/fleets/linux-build", (route) => {
    if (route.request().method() === "GET")
      return route.fulfill({ json: current, headers: { etag: '"original:2"' } });
    writes += 1;
    expect(route.request().postDataJSON().template_inputs).toEqual({
      runner_image: "current-image",
    });
    expect(route.request().postDataJSON().template_profile_ref).toEqual(
      current.spec.template_profile_ref,
    );
    return route.fulfill({
      status: 202,
      json: { changeId: "replaced", state: "Accepted", revision: 3 },
    });
  });
  await page.goto("/fleets/linux-build");
  await page.getByRole("button", { name: "Edit fleet" }).click();
  await page.getByRole("button", { name: "Remove obsolete", exact: true }).click();
  // The invalid existing-value option may precede the approved values.
  const image = page.getByRole("combobox", { name: "Runner image", exact: true });
  const approved = image.locator("option").filter({ hasText: "current-image" });
  await image.selectOption((await approved.getAttribute("value")) as string);
  await page.getByRole("button", { name: "Save changes" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(writes).toBe(1);
});

test("an existing bare-key fleet reads its resolved historical contract without repinning", async ({
  page,
}) => {
  await mockApi(page);
  const current = fleet("linux-build");
  const historical = {
    ...current,
    spec: {
      ...current.spec,
      template_profile_ref: "kubernetes-linux",
      template_inputs: { runner_image: "historical-image" },
    },
  };
  await mockContract(
    page,
    contract([field("runner_image", ['"historical-image"'], true, "Runner image")]),
  );
  const reads: string[] = [];
  page.on("request", (request) => {
    if (request.url().endsWith("/input-contract")) reads.push(request.url());
  });
  await page.route("**/api/v1/template-profiles/kubernetes-linux", (route) =>
    route.fulfill({
      json: {
        key: "kubernetes-linux",
        incarnation: "template-inc",
        desiredRevision: 4,
        activeRevision: 4,
        status: "Active",
      },
    }),
  );
  await page.route("**/api/v1/fleets/linux-build", (route) => {
    if (route.request().method() === "GET")
      return route.fulfill({ json: historical, headers: { etag: '"old:2"' } });
    expect(route.request().postDataJSON().template_profile_ref).toBe("kubernetes-linux");
    expect(route.request().postDataJSON().template_inputs).toEqual({
      runner_image: "historical-image",
    });
    return route.fulfill({
      status: 202,
      json: { changeId: "historical", state: "Accepted", revision: 3 },
    });
  });
  await page.goto("/fleets/linux-build");
  await page.getByRole("button", { name: "Edit fleet" }).click();
  await expect(page.getByRole("combobox", { name: "Runner image", exact: true })).not.toHaveValue(
    "",
  );
  await page.getByRole("button", { name: "Save changes" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(reads.length).toBeGreaterThan(0);
  for (const path of reads) expect(path).toContain("/revisions/3/input-contract");
});

test("a historical value exceeding the visual projection depth still permits unchanged input preservation", async ({
  page,
}) => {
  await mockApi(page);
  const current = fleet("linux-build");
  const nestedJson = `${"[".repeat(70)}9007199254740993${"]".repeat(70)}`;
  const body = JSON.stringify({
    ...current,
    spec: { ...current.spec, template_inputs: { nested: "DEEP_VALUE" } },
  }).replace('"DEEP_VALUE"', nestedJson);
  await page.route("**/api/v1/template-profiles/**/input-contract", (route) =>
    route.fulfill({
      status: 409,
      json: {
        code: "InputContractUnavailable",
        detail: "Input contract exceeds the maximum display depth",
      },
    }),
  );
  await page.route("**/api/v1/fleets/linux-build", (route) => {
    if (route.request().method() === "GET")
      return route.fulfill({
        body,
        contentType: "application/json",
        headers: { etag: '"original:2"' },
      });
    expect(route.request().postData()).toContain(`"nested":${nestedJson}`);
    expect(route.request().postDataJSON().capacity.max_runners).toBe(12);
    return route.fulfill({
      status: 202,
      json: { changeId: "depth-preserved", state: "Accepted", revision: 3 },
    });
  });
  await page.goto("/fleets/linux-build");
  await page.getByRole("button", { name: "Edit fleet" }).click();
  await expect(page.getByRole("dialog")).toContainText("maximum display depth");
  await page.getByLabel("Maximum runners", { exact: true }).fill("12");
  await page.getByRole("button", { name: "Save changes" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
});
