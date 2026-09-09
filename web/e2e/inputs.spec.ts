import { expect, test } from "@playwright/test";

test("real daemon activates a legacy Ready template and loads its approved Fleet inputs", async ({
  page,
  request,
}) => {
  await request.post("/__test/provider", { data: {} });
  const path = "/api/v1/template-profiles/browser-inputs/revisions/1/input-contract";
  expect((await request.get(path)).status()).toBe(401);
  await page.goto("/fleets");
  await expect(page.getByRole("heading", { name: "Fleets", exact: true })).toBeVisible();
  // The SQLite fixture starts at Ready with no activation ID or attestation.
  // Only the production daemon scan may make it available to this editor.
  await expect
    .poll(async () => {
      const profile = await page.request.get("/api/v1/template-profiles/browser-inputs");
      return (await profile.json()).activeRevision;
    })
    .toBe(1);
  const revision = await page.request.get("/api/v1/template-profiles/browser-inputs/revisions/1");
  expect(await revision.json()).toMatchObject({ state: "Active", reason: null });
  const contract = await page.request.get(path);
  expect(contract.status()).toBe(200);
  expect(contract.headers()["cache-control"]).toBe("private, no-store");
  const data = await contract.json();
  expect(data).toMatchObject({
    version: 1,
    profileKey: "browser-inputs",
    incarnation: "browser-template-incarnation",
    revision: 1,
    mode: "fields",
  });
  expect(
    data.fields.find((field: { key: string }) => field.key === "runner_image").options,
  ).toEqual([{ valueJson: '"runner:approved"' }]);
  expect(JSON.stringify(data)).not.toContain("runner:unapproved");

  const authList = page.waitForResponse((response) =>
    response.url().endsWith("/api/v1/github-auth-profiles"),
  );
  const templateList = page.waitForResponse((response) =>
    response.url().endsWith("/api/v1/template-profiles"),
  );
  await page
    .getByRole("region", { name: "Fleet inventory" })
    .getByRole("button", { name: "Create fleet", exact: true })
    .click();
  const dialog = page.getByRole("dialog");
  for (const [response, key] of [
    [await authList, "browser-auth"],
    [await templateList, "browser-inputs"],
  ] as const) {
    expect(response.status()).toBe(200);
    expect(response.headers()["cache-control"]).toBe("private, no-store");
    const collection = await response.json();
    expect(collection.profiles).toEqual(
      expect.arrayContaining([expect.objectContaining({ key, activeRevision: 1 })]),
    );
    expect(JSON.stringify(collection)).not.toContain("browser-fixture-inert-credential");
  }
  await dialog.getByLabel("Fleet key", { exact: true }).fill("browser-visual-inputs");
  await dialog.getByLabel("Owner", { exact: true }).fill("example-org");
  const authentication = dialog.getByRole("combobox", {
    name: "GitHub authentication profile",
    exact: true,
  });
  const template = dialog.getByRole("combobox", { name: "Template profile", exact: true });
  await expect(authentication).toHaveValue("");
  await expect(template).toHaveValue("");
  await authentication.selectOption("browser-auth");
  await template.selectOption("browser-inputs");
  const image = dialog.getByRole("combobox", { name: "Runner image", exact: true });
  await expect(image).toBeVisible();
  await expect(image).toHaveValue("");
  await image.selectOption({ label: "runner:approved" });
  await expect(dialog.getByRole("combobox", { name: "CPU request", exact: true })).toBeVisible();
  await dialog
    .getByRole("combobox", { name: "CPU request", exact: true })
    .selectOption({ label: "1" });
  await dialog.getByRole("button", { name: "Advanced settings", exact: true }).click();
  await dialog.getByRole("button", { name: "Advanced settings", exact: true }).click();
  await expect(dialog.getByRole("combobox", { name: "CPU request", exact: true })).toBeVisible();

  const submitted = page.waitForResponse(
    (response) =>
      response.url().endsWith("/api/v1/fleets/browser-visual-inputs") &&
      response.request().method() === "PUT",
  );
  await dialog.getByRole("button", { name: "Create fleet", exact: true }).click();
  const response = await submitted;
  expect(response.request().postDataJSON()).toMatchObject({
    github: { auth_profile_ref: "browser-auth" },
    template_profile_ref: { key: "browser-inputs", revision: 1 },
    template_inputs: { runner_image: "runner:approved", cpu_request: "1" },
  });
  // A selectable Profile is not a target authorization guarantee. The inert
  // fixture denies this target locally, before any Fleet or GitHub effect.
  expect(response.status()).toBe(422);
  expect(await response.json()).toMatchObject({
    code: "Unprocessable",
    detail: "auth profile target policy does not cover the fleet target",
  });
  expect((await page.request.get("/api/v1/fleets/browser-visual-inputs")).status()).toBe(404);
  await expect(image.locator("option:checked")).toHaveText("runner:approved");
  await expect(authentication).toHaveValue("browser-auth");
  await expect(template).toHaveValue("browser-inputs");
  await expect(dialog).toBeVisible();
});
