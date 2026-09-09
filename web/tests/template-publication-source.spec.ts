import { createHash } from "node:crypto";
import { expect, test } from "@playwright/test";
import { dockerDigest, mockTemplateLibrary } from "./template-library-fixtures";
import { mockTemplateUpdate } from "./template-update-fixtures";

for (const existing of [false, true]) {
  test(`Default publication records its source and fixes the engine (new revision: ${existing})`, async ({
    page,
  }) => {
    await mockTemplateUpdate(page);
    const key = existing ? "custom-docker" : "from-default";
    await page.route(`**/api/v1/template-profiles/${key}`, (route) => {
      if (route.request().method() === "GET") return route.fallback();
      expect(route.request().headers()[existing ? "if-match" : "if-none-match"]).toBe(
        existing ? '"template-inc:3"' : "*",
      );
      expect(route.request().postDataJSON()).toEqual({
        artifact_digest: dockerDigest,
        engine_ref: "terraform",
        source_key: "docker",
        bindings: {},
        fleet_input_policy: {},
      });
      return route.fulfill({
        status: 202,
        json: { changeId: "published-source", state: "Accepted", revision: existing ? 4 : 1 },
      });
    });
    await page.goto(existing ? `/templates/${key}/revisions/new` : "/templates/new");
    if (!existing) await page.getByLabel("Profile key").fill(key);
    await page.getByRole("button", { name: "Advanced settings" }).click();
    const engine = page.getByLabel("Engine reference", { exact: true });
    await engine.fill("custom-engine");
    await page.getByRole("radio", { name: "Default template", exact: true }).check();
    const source = page.getByRole("combobox", { name: "Default template", exact: true });
    await source.selectOption("docker-alternative");
    await expect(engine).toHaveValue("opentofu");
    await expect(engine).not.toBeEditable();
    await source.selectOption("docker");
    await expect(engine).toHaveValue("terraform");
    await expect(engine).not.toBeEditable();
    await page.getByRole("button", { name: "Publish", exact: true }).click();
    await expect(page).toHaveURL(new RegExp(`/templates\\?key=${key}$`));
  });
}

for (const kind of ["archive", "existing"]) {
  test(`switching from Default to ${kind} omits the old source key`, async ({ page }) => {
    await mockTemplateLibrary(page);
    const bytes = Buffer.from("customized-template");
    const digest =
      kind === "archive"
        ? `sha256:${createHash("sha256").update(bytes).digest("hex")}`
        : dockerDigest;
    const key = `switched-${kind}`;
    await page.route("**/api/v1/template-artifacts/*", (route) =>
      route.fulfill({ status: 201, json: { digest } }),
    );
    await page.route(`**/api/v1/template-profiles/${key}`, (route) => {
      if (route.request().method() === "GET") return route.fallback();
      expect(route.request().postDataJSON()).toEqual({
        artifact_digest: digest,
        engine_ref: "custom-engine",
        bindings: {},
        fleet_input_policy: {},
      });
      return route.fulfill({
        status: 202,
        json: { changeId: "unassociated", state: "Accepted", revision: 1 },
      });
    });
    await page.goto("/templates");
    await page.getByRole("button", { name: "Use template docker" }).click();
    await page.getByLabel("Profile key").fill(key);
    await page
      .getByRole("radio", {
        name: kind === "archive" ? "Upload archive" : "Existing artifact",
        exact: true,
      })
      .check();
    if (kind === "archive")
      await page
        .getByLabel("Template archive (.tar.gz)")
        .setInputFiles({ name: "custom.tar.gz", mimeType: "application/gzip", buffer: bytes });
    else await page.getByLabel("Existing artifact digest", { exact: true }).fill(digest);
    await page.getByRole("button", { name: "Advanced settings" }).click();
    const engine = page.getByLabel("Engine reference", { exact: true });
    await expect(engine).toBeEditable();
    await engine.fill("custom-engine");
    await page.getByRole("button", { name: "Publish", exact: true }).click();
    await expect(page).toHaveURL(new RegExp(`/templates\\?key=${key}$`));
  });
}
