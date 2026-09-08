import { expect, test } from "@playwright/test";

test("real daemon projects approved inputs into the authenticated Fleet editor", async ({
  page,
  request,
}) => {
  await request.post("/__test/provider", { data: {} });
  const path = "/api/v1/template-profiles/browser-inputs/revisions/1/input-contract";
  expect((await request.get(path)).status()).toBe(401);
  await page.goto("/fleets");
  await expect(page.getByRole("heading", { name: "Fleets", exact: true })).toBeVisible();
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

  await page
    .getByRole("region", { name: "Fleet inventory" })
    .getByRole("button", { name: "Create fleet", exact: true })
    .click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Fleet key", { exact: true }).fill("browser-visual-inputs");
  await dialog.getByLabel("Owner", { exact: true }).fill("example-org");
  await dialog.getByLabel("GitHub authentication profile", { exact: true }).fill("missing-auth");
  await dialog.getByLabel("Template profile", { exact: true }).fill("browser-inputs");
  await dialog.getByLabel("Template profile", { exact: true }).press("Tab");
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
    template_profile_ref: { key: "browser-inputs", revision: 1 },
    template_inputs: { runner_image: "runner:approved", cpu_request: "1" },
  });
  // Real admission reaches the deliberately absent Auth Profile without creating a Fleet.
  expect(response.status()).toBe(422);
  expect(await response.json()).toMatchObject({
    code: "Unprocessable",
    detail: "auth profile missing-auth missing",
  });
  expect((await page.request.get("/api/v1/fleets/browser-visual-inputs")).status()).toBe(404);
  await expect(image.locator("option:checked")).toHaveText("runner:approved");
  await expect(dialog).toBeVisible();
});
