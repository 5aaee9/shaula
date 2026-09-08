import { createHash } from "node:crypto";
import { expect, test } from "@playwright/test";
import { fleet, mockApi } from "./fixtures";

test("collapsed fleet settings preserve the edit snapshot and reveal invalid fields", async ({
  page,
}) => {
  await mockApi(page);
  const current = fleet("linux-build");
  current.spec.github.runner_group = "Builders";
  current.spec.github.labels = ["arm64"];
  current.spec.capacity.min_runners = 2;
  current.spec.template_profile_ref.revision = 7;
  current.spec.template_inputs = { machine: "arm64" };
  let writes = 0;
  await page.route("**/api/v1/fleets/linux-build", (route) => {
    if (route.request().method() === "GET")
      return route.fulfill({ headers: { etag: '"original:2"' }, json: current });
    writes += 1;
    expect(route.request().headers()["if-match"]).toBe('"original:2"');
    expect(route.request().postDataJSON()).toEqual(current.spec);
    return route.fulfill({
      status: 202,
      json: { changeId: "edited", state: "Accepted", revision: 3 },
    });
  });
  await page.goto("/fleets/linux-build");
  await page.getByRole("button", { name: "Edit fleet" }).click();
  const advanced = page.getByRole("button", { name: "Advanced settings" });
  const save = page.getByRole("button", { name: "Save changes" });
  await expect(advanced).toHaveAttribute("aria-expanded", "false");
  await advanced.click();
  await expect(page.getByLabel("Runner group")).toHaveValue("Builders");
  await page.getByLabel("Template inputs (JSON)").fill("[]");
  await advanced.click();
  await save.click();
  await expect(advanced).toHaveAttribute("aria-expanded", "true");
  await expect(page.getByRole("alert")).toContainText("Template inputs must be a JSON object");
  await page
    .getByLabel("Template inputs (JSON)")
    .fill(JSON.stringify(current.spec.template_inputs));
  await page.getByLabel("Pinned revision").fill("0");
  await advanced.click();
  await save.click();
  await expect(page.getByLabel("Pinned revision")).toBeFocused();
  await expect(advanced).toHaveAttribute("aria-expanded", "true");
  expect(writes).toBe(0);
  await page.getByLabel("Pinned revision").fill("7");
  await advanced.click();
  await save.click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(writes).toBe(1);
});

test("template upload keeps source choices and reveals invalid advanced settings", async ({
  page,
}) => {
  await mockApi(page);
  await page.setViewportSize({ width: 390, height: 844 });
  const bytes = Buffer.from("test-template-archive");
  const digest = `sha256:${createHash("sha256").update(bytes).digest("hex")}`;
  let uploads = 0;
  let publications = 0;
  await page.route("**/api/v1/template-artifacts/*", (route) => {
    uploads += 1;
    expect(route.request().url()).toContain(encodeURIComponent(digest));
    expect(route.request().postDataBuffer()).toEqual(bytes);
    return route.fulfill({ status: 201, json: { digest } });
  });
  await page.route("**/api/v1/template-profiles/uploaded", (route) => {
    publications += 1;
    expect(route.request().headers()["if-none-match"]).toBe("*");
    expect(route.request().postDataJSON()).toEqual({
      artifact_digest: digest,
      engine_ref: "terraform",
      bindings: {},
      fleet_input_policy: {},
    });
    return route.fulfill({
      status: 202,
      json: { changeId: "uploaded", state: "Accepted", revision: 1 },
    });
  });
  await page.goto("/templates");
  await page.getByRole("button", { name: "Publish template" }).click();
  await page.getByLabel("Profile key").fill("uploaded");
  const file = page.getByLabel("Template archive (.tar.gz)");
  await file.setInputFiles({ name: "runner.tar.gz", mimeType: "application/gzip", buffer: bytes });
  const advanced = page.getByRole("button", { name: "Advanced settings" });
  await expect(advanced).toHaveAttribute("aria-expanded", "false");
  await expect(page.getByLabel("Bindings (JSON)")).toBeHidden();
  await expect(page.getByLabel("Existing artifact digest")).toBeHidden();
  await page.screenshot({
    path: "test-results/template-dialog-compact.png",
    fullPage: true,
    animations: "disabled",
  });
  expect(
    await page.getByRole("dialog").evaluate((node) => node.scrollWidth <= node.clientWidth),
  ).toBe(true);
  await advanced.click();
  await page.getByLabel("Engine reference").fill("");
  await advanced.click();
  await page.getByRole("button", { name: "Publish", exact: true }).click();
  await expect(page.getByLabel("Engine reference")).toBeFocused();
  await page.getByLabel("Engine reference").fill("terraform");
  await page.getByLabel("Bindings (JSON)").fill("[]");
  await advanced.click();
  await page.getByRole("button", { name: "Publish", exact: true }).click();
  await expect(page.getByLabel("Bindings (JSON)")).toBeVisible();
  await expect(page.getByRole("alert")).toContainText("Bindings must be a JSON object");
  expect(uploads).toBe(0);
  await page.getByLabel("Bindings (JSON)").fill("{}");
  await advanced.click();
  await page.getByLabel("Template source").selectOption("existing");
  await page.getByLabel("Existing artifact digest").fill(`sha256:${"a".repeat(64)}`);
  await page.getByLabel("Template source").selectOption("archive");
  await expect(file).toHaveValue(/runner\.tar\.gz$/);
  await page.getByRole("button", { name: "Publish", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(uploads).toBe(1);
  expect(publications).toBe(1);
});
