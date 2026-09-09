import { expect, test } from "@playwright/test";
import { profile, scopes } from "./fixtures";
import { dockerDigest } from "./template-library-fixtures";
import {
  currentDigest,
  mockTemplateUpdate,
  updateApi,
  updatePath,
  updateSources,
} from "./template-update-fixtures";

test("published update reviews an explicit source and retains bindings and policy without sending them", async ({
  page,
}) => {
  await mockTemplateUpdate(page);
  let updates = 0;
  let fleetWrites = 0;
  await page.route("**/api/v1/fleets/**", (route) => {
    if (route.request().method() !== "GET") fleetWrites++;
    return route.fallback();
  });
  await page.route(updateApi, (route) => {
    updates++;
    expect(route.request().method()).toBe("POST");
    expect(route.request().headers()["if-match"]).toBe('"template-inc:3"');
    expect(route.request().headers()["if-none-match"]).toBeUndefined();
    expect(route.request().headers()["idempotency-key"]).toBeTruthy();
    expect(route.request().postDataJSON()).toEqual({
      artifact_digest: dockerDigest,
      engine_ref: "terraform",
      source_key: "docker",
    });
    return route.fulfill({
      status: 202,
      json: { changeId: "template-updated", state: "Accepted", revision: 4 },
    });
  });
  await page.goto("/templates");
  await page.getByRole("button", { name: "Update custom-docker from default" }).click();
  await expect(page).toHaveURL(new RegExp(`${updatePath}$`));
  const form = page.getByRole("form", { name: "Template update review" });
  const source = form.getByRole("combobox", { name: "Default template", exact: true });
  await expect(source).toHaveValue("");
  await expect(source.getByRole("option", { name: "kubernetes", exact: true })).toHaveCount(0);
  await expect(form.getByRole("button", { name: "Update", exact: true })).toBeDisabled();
  await expect(form.getByRole("region", { name: "Current revision", exact: true })).toContainText(
    currentDigest,
  );
  await source.selectOption("docker");
  await expect(form.getByRole("button", { name: "Use declared options" })).toBeVisible();
  await expect(form.getByRole("region", { name: "Retained bindings" })).toContainText(
    "reuses the bindings from revision r3",
  );
  await expect(form.getByRole("region", { name: "Fleet input policy", exact: true })).toContainText(
    "Retain the policy",
  );
  await expect(form.getByLabel("Access token", { exact: true })).toHaveCount(0);
  await expect(form).not.toContainText("never-display-this");
  expect(updates).toBe(0);
  await form.getByRole("button", { name: "Update", exact: true }).click();
  await expect(page).toHaveURL(/\/templates\?key=custom-docker$/);
  await expect(page.getByText("Change for custom-docker")).toBeVisible();
  expect(updates).toBe(1);
  expect(fleetWrites).toBe(0);
});

test("explicit declared options replace policy while preserving exact numbers and excluding sensitive options", async ({
  page,
}) => {
  await mockTemplateUpdate(page);
  let written = "";
  await page.route(updateApi, (route) => {
    written = route.request().postData()!;
    return route.fulfill({
      status: 202,
      json: { changeId: "policy-updated", state: "Accepted", revision: 4 },
    });
  });
  await page.goto(updatePath);
  await page.getByLabel("Default template", { exact: true }).selectOption("docker");
  await page.getByRole("button", { name: "Use declared options" }).click();
  await expect(page.getByRole("region", { name: "Fleet input policy", exact: true })).toContainText(
    "Replace the entire policy",
  );
  await expect(page.getByLabel("Replacement policy")).toContainText("9007199254740993");
  await page.getByRole("button", { name: "Update", exact: true }).click();
  await expect(page).toHaveURL(/\/templates\?key=custom-docker$/);
  expect(written).toBe(
    `{"artifact_digest":"${dockerDigest}","engine_ref":"terraform","source_key":"docker","fleet_input_policy":{"runner_image":["runner:stable","runner:canary"],"quota":[9007199254740993]}}`,
  );
});

test("source changes require another explicit policy adoption", async ({ page }) => {
  await mockTemplateUpdate(page);
  await page.route(updateApi, (route) => {
    expect(route.request().postDataJSON()).toEqual({
      artifact_digest: dockerDigest,
      engine_ref: "opentofu",
      source_key: "docker-alternative",
    });
    return route.fulfill({
      status: 202,
      json: { changeId: "retained", state: "Accepted", revision: 4 },
    });
  });
  await page.goto(updatePath);
  const source = page.getByLabel("Default template", { exact: true });
  await source.selectOption("docker");
  await page.getByRole("button", { name: "Use declared options" }).click();
  await source.selectOption("docker-alternative");
  await expect(page.getByRole("region", { name: "Fleet input policy", exact: true })).toContainText(
    "Retain the policy",
  );
  await page.getByRole("button", { name: "Update", exact: true }).click();
  await expect(page).toHaveURL(/\/templates\?key=custom-docker$/);
});

test("an unlinked identical artifact can establish its source or review another engine", async ({
  page,
}) => {
  await mockTemplateUpdate(page, { digest: dockerDigest });
  await page.goto(updatePath);
  const source = page.getByLabel("Default template", { exact: true });
  await expect(source).toHaveValue("docker");
  await expect(page.getByRole("status").filter({ hasText: "Up to date" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Update", exact: true })).toBeEnabled();
  await source.selectOption("docker-alternative");
  await expect(page.getByRole("status").filter({ hasText: "Up to date" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Update", exact: true })).toBeEnabled();
});

for (const status of [409, 412]) {
  test(`a ${status} preserves the review draft and does not refresh or retry its base`, async ({
    page,
  }) => {
    await mockTemplateUpdate(page);
    let writes = 0;
    await page.route(updateApi, (route) => {
      writes++;
      expect(route.request().headers()["if-match"]).toBe('"template-inc:3"');
      return route.fulfill({
        status,
        json: {
          code: status === 412 ? "PreconditionFailed" : "IdempotencyConflict",
          detail: "The update conflicts with another request.",
        },
      });
    });
    await page.goto(updatePath);
    await page.getByLabel("Default template", { exact: true }).selectOption("docker");
    await page.getByRole("button", { name: "Use declared options" }).click();
    await page.getByRole("button", { name: "Update", exact: true }).click();
    await expect(
      page.getByRole("status").filter({ hasText: "Your draft is retained" }),
    ).toBeVisible();
    await expect(page.getByLabel("Default template", { exact: true })).toHaveValue("docker");
    await expect(page.getByLabel("Replacement policy")).toContainText("runner:stable");
    await expect(page.getByRole("button", { name: "Update", exact: true })).toBeDisabled();
    await page.evaluate(() => window.dispatchEvent(new Event("visibilitychange")));
    await expect(page.getByRole("region", { name: "Current revision", exact: true })).toContainText(
      "r3",
    );
    expect(writes).toBe(1);
  });
}

test("retrying an uncertain write retains its exact idempotency key and reviewed source", async ({
  page,
}) => {
  await mockTemplateUpdate(page);
  const requests: { key: string; body: string }[] = [];
  await page.route(updateApi, (route) => {
    requests.push({
      key: route.request().headers()["idempotency-key"],
      body: route.request().postData()!,
    });
    return requests.length === 1
      ? route.fulfill({
          status: 503,
          json: { code: "Unavailable", detail: "Temporarily unavailable" },
        })
      : route.fulfill({ status: 202, json: { changeId: "retry", state: "Accepted", revision: 4 } });
  });
  await page.goto(updatePath);
  await page.getByLabel("Default template", { exact: true }).selectOption("docker");
  await page.getByRole("button", { name: "Update", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("Temporarily unavailable");
  await page.route("**/api/v1/template-sources", (route) => {
    return route.fulfill({
      json: {
        sources: updateSources.map((source) => ({ ...source, artifactDigest: currentDigest })),
      },
    });
  });
  await page.evaluate(() => window.dispatchEvent(new Event("visibilitychange")));
  await expect(page.getByRole("region", { name: "Target revision", exact: true })).toContainText(
    dockerDigest,
  );
  await page.getByRole("button", { name: "Update", exact: true }).click();
  await expect(page).toHaveURL(/\/templates\?key=custom-docker$/);
  expect(requests).toHaveLength(2);
  expect(requests[1]).toEqual(requests[0]);
});

test("starting a new review after a conflict loads a new base instead of cached draft data", async ({
  page,
}) => {
  await mockTemplateUpdate(page);
  let writes = 0;
  await page.route(updateApi, (route) => {
    writes++;
    expect(route.request().headers()["if-match"]).toBe(
      writes === 1 ? '"template-inc:3"' : '"template-inc:4"',
    );
    return writes === 1
      ? route.fulfill({ status: 412, json: { code: "PreconditionFailed" } })
      : route.fulfill({
          status: 202,
          json: { changeId: "new-base", state: "Accepted", revision: 5 },
        });
  });
  await page.goto(updatePath);
  await page.getByLabel("Default template", { exact: true }).selectOption("docker");
  await page.getByRole("button", { name: "Update", exact: true }).click();
  await expect(
    page.getByRole("status").filter({ hasText: "Your draft is retained" }),
  ).toBeVisible();
  await page.route("**/api/v1/template-profiles/custom-docker", (route) =>
    route.fulfill({
      headers: { etag: '"template-inc:4"' },
      json: {
        ...profile("custom-docker", 4),
        desiredRevision: 4,
        platform: "docker",
        bindings_present: true,
      },
    }),
  );
  await page.route("**/api/v1/template-profiles/custom-docker/revisions/4", (route) =>
    route.fulfill({
      json: {
        revision: 4,
        artifactDigest: currentDigest,
        engineRef: "terraform",
        platform: "docker",
        state: "Active",
        reason: null,
      },
    }),
  );
  await page.getByRole("link", { name: "Back to templates" }).click();
  await page.getByRole("button", { name: "Update custom-docker from default" }).click();
  await expect(page.getByRole("region", { name: "Current revision", exact: true })).toContainText(
    "r4",
  );
  await expect(page.getByLabel("Default template", { exact: true })).toHaveValue("");
  await page.getByLabel("Default template", { exact: true }).selectOption("docker");
  await page.getByRole("button", { name: "Update", exact: true }).click();
  await expect(page).toHaveURL(/\/templates\?key=custom-docker$/);
  expect(writes).toBe(2);
});

test("the update review keeps target and policy values readable on desktop and mobile", async ({
  page,
}) => {
  await mockTemplateUpdate(page);
  await page.goto(updatePath);
  await page.getByLabel("Default template", { exact: true }).selectOption("docker");
  await page.getByRole("button", { name: "Use declared options" }).click();
  await page.screenshot({
    path: "test-results/template-update-desktop.png",
    fullPage: true,
    animations: "disabled",
  });
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(page.getByLabel("Replacement policy")).toContainText("9007199254740993");
  await page
    .getByRole("heading", { name: "Update custom-docker from default" })
    .scrollIntoViewIfNeeded();
  await expect(page.getByRole("button", { name: "Update", exact: true })).toBeInViewport({
    ratio: 1,
  });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(
    true,
  );
  await page.screenshot({
    path: "test-results/template-update-mobile.png",
    fullPage: true,
    animations: "disabled",
  });
});

for (const missing of ["template.publish", "template.read"]) {
  test(`update review requires ${missing}`, async ({ page }) => {
    await mockTemplateUpdate(page, { permissions: scopes.filter((scope) => scope !== missing) });
    let reads = 0;
    await page.route("**/api/v1/template-profiles/custom-docker**", (route) => {
      reads++;
      return route.fallback();
    });
    await page.goto(updatePath);
    await expect(page.getByRole("alert")).toContainText(
      "Template read and publish permissions are required",
    );
    await expect(page.getByRole("form", { name: "Template update review" })).toHaveCount(0);
    expect(reads).toBe(0);
    if (missing === "template.publish") {
      await page.goto("/templates?key=custom-docker");
      await expect(
        page.getByRole("button", { name: "Update custom-docker from default" }),
      ).toBeDisabled();
      await expect(
        page.getByRole("button", { name: "Update from default", exact: true }),
      ).toBeDisabled();
    }
  });
}

test("published template detail actions fit a 390px viewport", async ({ page }) => {
  await mockTemplateUpdate(page);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/templates?key=custom-docker");
  const update = page.getByRole("button", { name: "Update from default", exact: true });
  await update.scrollIntoViewIfNeeded();
  await expect(update).toBeVisible();
  const bounds = await update.evaluate((button) => {
    const group = button.parentElement!;
    const section = group.closest("section")!.getBoundingClientRect();
    return {
      documentWidth: document.documentElement.scrollWidth,
      viewportWidth: window.innerWidth,
      sectionRight: section.right,
      rightEdges: Array.from(group.querySelectorAll("button")).map(
        (item) => item.getBoundingClientRect().right,
      ),
    };
  });
  await page.screenshot({
    path: "test-results/template-detail-actions-mobile.png",
    fullPage: true,
    animations: "disabled",
  });
  expect(bounds.documentWidth, JSON.stringify(bounds)).toBeLessThanOrEqual(bounds.viewportWidth);
  for (const right of bounds.rightEdges)
    expect(right, JSON.stringify(bounds)).toBeLessThanOrEqual(bounds.sectionRight);
});

for (const status of ["Retiring", "Retired"]) {
  test(`${status} templates cannot enter an update through either entry point or direct navigation`, async ({
    page,
  }) => {
    await mockTemplateUpdate(page, { status });
    await page.goto("/templates?key=custom-docker");
    await expect(
      page.getByRole("button", { name: "Update custom-docker from default" }),
    ).toBeDisabled();
    await expect(
      page.getByRole("button", { name: "Update from default", exact: true }),
    ).toBeDisabled();
    await page.goto(updatePath);
    await expect(page.getByRole("alert")).toContainText(
      "retiring or retired template cannot be updated",
    );
    await expect(page.getByRole("form", { name: "Template update review" })).toHaveCount(0);
  });
}
