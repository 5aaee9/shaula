import { expect, test, type Page } from "@playwright/test";
import { scopes } from "./fixtures";
import { dockerDigest, sources } from "./template-library-fixtures";
import {
  currentDigest,
  mockTemplateUpdate,
  updateApi,
  updatePath,
  updateSources,
} from "./template-update-fixtures";

for (const source of sources) {
  test(`${source.platform} updates use the saved source after its artifact changes`, async ({
    page,
  }, testInfo) => {
    await mockTemplateUpdate(page, {
      platform: source.platform,
      sourceKey: source.key,
      sources: [
        { ...source, key: `${source.key}-alternative`, artifactDigest: currentDigest },
        source,
      ],
    });
    let updates = 0;
    await page.route(updateApi, (route) => {
      updates++;
      expect(route.request().headers()["if-match"]).toBe('"template-inc:3"');
      expect(route.request().postDataJSON()).toEqual({
        artifact_digest: source.artifactDigest,
        engine_ref: source.engineRef,
        source_key: source.key,
      });
      return route.fulfill({
        status: 202,
        json: { changeId: "saved-source", state: "Accepted", revision: 4 },
      });
    });
    await page.goto("/templates");
    await page.getByRole("button", { name: "Update custom-docker from default" }).click();
    await expect(page.getByRole("combobox", { name: "Default template", exact: true })).toHaveCount(
      0,
    );
    await expect(page.getByRole("region", { name: "Target revision", exact: true })).toContainText(
      source.artifactDigest,
    );
    await expect(page.getByRole("region", { name: "Update source" })).toContainText(source.key);
    if (source.key === "docker") {
      await page.screenshot({
        path: testInfo.outputPath("saved-template-source-desktop.png"),
        fullPage: true,
      });
      await page.setViewportSize({ width: 390, height: 844 });
      expect(
        await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth),
      ).toBe(true);
      await page.screenshot({
        path: testInfo.outputPath("saved-template-source-mobile.png"),
        fullPage: true,
      });
    }
    expect(updates).toBe(0);
    await page.getByRole("button", { name: "Update", exact: true }).click();
    await expect(page).toHaveURL(/\/templates\?key=custom-docker$/);
    expect(updates).toBe(1);
  });

  test(`an unlinked ${source.platform} revision preselects its only platform candidate`, async ({
    page,
  }) => {
    await mockTemplateUpdate(page, { platform: source.platform, sources });
    await page.goto(updatePath);
    await expect(page.getByLabel("Default template", { exact: true })).toHaveValue(source.key);
    await expect(page.getByRole("button", { name: "Update", exact: true })).toBeEnabled();
    await expect(page.getByRole("region", { name: "Target revision", exact: true })).toContainText(
      source.artifactDigest,
    );
  });
}

test("an unlinked revision matches both artifact and engine before persisting the selected source", async ({
  page,
}) => {
  await mockTemplateUpdate(page, { digest: dockerDigest, sources: [...updateSources].reverse() });
  await page.route(updateApi, (route) => {
    expect(route.request().postDataJSON()).toEqual({
      artifact_digest: dockerDigest,
      engine_ref: "terraform",
      source_key: "docker",
    });
    return route.fulfill({
      status: 202,
      json: { changeId: "source-linked", state: "Accepted", revision: 4 },
    });
  });
  await page.goto(updatePath);
  await expect(page.getByLabel("Default template", { exact: true })).toHaveValue("docker");
  await expect(page.getByRole("status").filter({ hasText: "Up to date" })).toHaveCount(0);
  await page.getByRole("button", { name: "Update", exact: true }).click();
  await expect(page).toHaveURL(/\/templates\?key=custom-docker$/);
});

test("a saved source with the same artifact and engine is already up to date", async ({ page }) => {
  await mockTemplateUpdate(page, { digest: dockerDigest, sourceKey: "docker" });
  await page.goto(updatePath);
  await expect(page.getByRole("status").filter({ hasText: "Up to date" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Update", exact: true })).toBeDisabled();
});

test("an unchanged saved source still permits explicit policy replacement", async ({ page }) => {
  await mockTemplateUpdate(page, { digest: dockerDigest, sourceKey: "docker" });
  await page.route(updateApi, (route) => {
    const body = route.request().postDataJSON();
    expect(body.source_key).toBe("docker");
    expect(body.artifact_digest).toBe(dockerDigest);
    expect(body.engine_ref).toBe("terraform");
    expect(body.fleet_input_policy.runner_image).toEqual(["runner:stable", "runner:canary"]);
    return route.fulfill({
      status: 202,
      json: { changeId: "policy-only", state: "Accepted", revision: 4 },
    });
  });
  await page.goto(updatePath);
  const update = page.getByRole("button", { name: "Update", exact: true });
  await expect(update).toBeDisabled();
  await page.getByRole("button", { name: "Use declared options" }).click();
  await expect(update).toBeEnabled();
  await page.getByRole("button", { name: "Retain current policy" }).click();
  await expect(update).toBeDisabled();
  await page.getByRole("button", { name: "Use declared options" }).click();
  await update.click();
  await expect(page).toHaveURL(/\/templates\?key=custom-docker$/);
});

test("a changed default rejected with 422 preserves the reviewed source until a new review", async ({
  page,
}) => {
  await mockTemplateUpdate(page, { sourceKey: "docker" });
  let writes = 0;
  await page.route(updateApi, (route) => {
    writes++;
    expect(route.request().postDataJSON().artifact_digest).toBe(dockerDigest);
    return route.fulfill({
      status: 422,
      json: { code: "Validation", detail: "The selected default template has changed." },
    });
  });
  await page.goto(updatePath);
  await page.getByRole("button", { name: "Use declared options" }).click();
  await page.route("**/api/v1/template-sources", (route) =>
    route.fulfill({ json: { sources: [] } }),
  );
  await page.getByRole("button", { name: "Update", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("The selected default template has changed");
  await expect(
    page.getByRole("status").filter({ hasText: "Your draft is retained" }),
  ).toBeVisible();
  await expect(page.getByRole("region", { name: "Target revision", exact: true })).toContainText(
    dockerDigest,
  );
  await expect(page.getByLabel("Replacement policy")).toContainText("9007199254740993");
  await expect(page.getByRole("button", { name: "Update", exact: true })).toBeDisabled();
  expect(writes).toBe(1);
  await page.getByRole("link", { name: "Back to templates" }).click();
  await page.getByRole("button", { name: "Update custom-docker from default" }).click();
  await expect(page.getByRole("status")).toContainText(
    "Saved default template docker is unavailable",
  );
});

for (const duplicateExactMatch of [false, true]) {
  test(`an unlinked ambiguous catalog does not guess a source (duplicate exact match: ${duplicateExactMatch})`, async ({
    page,
  }) => {
    await mockTemplateUpdate(page, {
      digest: duplicateExactMatch ? dockerDigest : currentDigest,
      sources: duplicateExactMatch
        ? [...sources, { ...sources[0], key: "docker-alias" }]
        : updateSources,
    });
    await page.goto(updatePath);
    await expect(page.getByLabel("Default template", { exact: true })).toHaveValue("");
    await expect(page.getByRole("button", { name: "Update", exact: true })).toBeDisabled();
  });
}

test("no platform candidates keeps an unlinked update unavailable", async ({ page }) => {
  await mockTemplateUpdate(page, { sources: [sources[1]] });
  await page.goto(updatePath);
  await expect(page.getByRole("status")).toContainText("No defaults are available");
  await expect(page.getByRole("button", { name: "Update", exact: true })).toBeDisabled();
});

for (const sourceKey of ["removed-source", "kubernetes"]) {
  test(`a missing or incompatible saved source never falls back (${sourceKey})`, async ({
    page,
  }) => {
    await mockTemplateUpdate(page, { sourceKey, digest: dockerDigest, sources });
    await page.goto(updatePath);
    await expect(page.getByRole("status")).toContainText(
      `Saved default template ${sourceKey} is unavailable for this platform`,
    );
    await expect(page.getByRole("combobox", { name: "Default template", exact: true })).toHaveCount(
      0,
    );
    await expect(page.getByRole("button", { name: "Update", exact: true })).toBeDisabled();
  });
}

async function refreshBackgroundQueries(page: Page, name: string) {
  await page.route("**/api/v1/session", (route) =>
    route.fulfill({ headers: { "x-csrf-token": "test-session-csrf" }, json: { name, scopes } }),
  );
  await page.clock.fastForward(3_001);
  await page.evaluate(() => window.dispatchEvent(new Event("visibilitychange")));
  await expect(page.getByText(name, { exact: true })).toBeVisible();
}

test("clearing or changing a legacy selection survives background refresh with its exact review", async ({
  page,
}) => {
  await page.clock.install();
  await mockTemplateUpdate(page, { digest: dockerDigest });
  let catalogReads = 0;
  let latestSources = updateSources;
  await page.route("**/api/v1/template-sources", (route) => {
    catalogReads++;
    return route.fulfill({ json: { sources: latestSources } });
  });
  await page.goto(updatePath);
  const select = page.getByLabel("Default template", { exact: true });
  await expect(select).toHaveValue("docker");
  const initialReads = catalogReads;
  await select.selectOption("");
  latestSources = updateSources.map((source) => ({ ...source, artifactDigest: currentDigest }));
  await refreshBackgroundQueries(page, "Refreshed after clearing");
  await expect(select).toHaveValue("");
  await expect(page.getByRole("button", { name: "Update", exact: true })).toBeDisabled();
  await select.selectOption("docker-alternative");
  await page.getByRole("button", { name: "Use declared options" }).click();
  await refreshBackgroundQueries(page, "Refreshed after choosing");
  await expect(select).toHaveValue("docker-alternative");
  await expect(page.getByLabel("Replacement policy")).toContainText("9007199254740993");
  await expect(page.getByRole("region", { name: "Target revision", exact: true })).toContainText(
    dockerDigest,
  );
  expect(catalogReads).toBe(initialReads);
  await page.route(updateApi, (route) => {
    expect(route.request().headers()["if-match"]).toBe('"template-inc:3"');
    const body = route.request().postDataJSON();
    expect(body.artifact_digest).toBe(dockerDigest);
    expect(body.engine_ref).toBe("opentofu");
    expect(body.source_key).toBe("docker-alternative");
    expect(body.fleet_input_policy.runner_image).toEqual(["runner:stable", "runner:canary"]);
    return route.fulfill({
      status: 202,
      json: { changeId: "frozen-source", state: "Accepted", revision: 4 },
    });
  });
  await page.getByRole("button", { name: "Update", exact: true }).click();
  await expect(page).toHaveURL(/\/templates\?key=custom-docker$/);
});
