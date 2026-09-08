import { createHash } from "node:crypto";
import { expect, test } from "@playwright/test";
import { mockApi, scopes } from "./fixtures";
import {
  dockerDigest,
  kubernetesDigest,
  mockTemplateLibrary,
  sources,
  variables,
} from "./template-library-fixtures";

test("library discovery displays variables but only explicit actions adopt defaults and policy", async ({
  page,
}) => {
  await mockTemplateLibrary(page);
  await page.goto("/templates");
  await expect(page.getByRole("region", { name: "Default templates" })).toContainText("docker");
  await page.getByRole("button", { name: "Use template docker" }).click();
  await expect(page.getByLabel("Template source", { exact: true })).toHaveValue("default");
  await expect(page.getByLabel("Docker host", { exact: true })).toHaveValue("");
  await expect(page.getByRole("region", { name: "Fleet input variables" })).toContainText(
    "runner:stable",
  );
  await page.getByRole("button", { name: "Advanced settings" }).click();
  await expect(page.getByLabel("Bindings (JSON)")).toHaveValue("{}");
  await expect(page.getByLabel("Fleet input policy (JSON)")).toHaveValue("{}");
  await page
    .getByLabel("Bindings (JSON)")
    .fill('{"docker_host":"tcp://mine:2375","__proto__":false}');
  await page.getByLabel("Fleet input policy (JSON)").fill('{"runner_image":["runner:stable"]}');
  await page.getByRole("button", { name: "Use defaults" }).click();
  await page.getByRole("button", { name: "Use declared options" }).click();
  await expect(page.getByLabel("Docker host", { exact: true })).toHaveValue("tcp://mine:2375");
  await expect(page.getByLabel("Quota", { exact: true })).toHaveValue("9007199254740993");
  await expect(page.getByLabel("Optional name", { exact: true })).toHaveValue("");
  await expect(page.getByLabel("Bindings (JSON)")).toHaveValue(
    '{"docker_host":"tcp://mine:2375","__proto__":false,"quota":9007199254740993}',
  );
  await expect(page.getByLabel("Fleet input policy (JSON)")).toHaveValue(
    '{"runner_image":["runner:stable"]}',
  );
  await page.getByLabel("Access token", { exact: true }).fill("private-draft");
  await expect(page.getByLabel("Access token", { exact: true })).toHaveAttribute(
    "type",
    "password",
  );
  expect(await page.evaluate(() => JSON.stringify({ ...localStorage }))).not.toContain(
    "private-draft",
  );
  await page.getByLabel("Profile key").fill("from-library");
  let written = "";
  await page.route("**/api/v1/template-profiles/from-library", (route) => {
    written = route.request().postData()!;
    expect(route.request().headers()["if-none-match"]).toBe("*");
    return route.fulfill({
      status: 202,
      json: { changeId: "library-published", state: "Accepted", revision: 1 },
    });
  });
  await page.getByRole("button", { name: "Publish", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(written).toContain('"quota":9007199254740993');
  expect(written).toContain('"__proto__":false');
  expect(JSON.parse(written).artifact_digest).toBe(dockerDigest);
});

test("declared options approval preserves exact values and variables stay outside advanced", async ({
  page,
}) => {
  await mockTemplateLibrary(page);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/templates");
  await page.getByRole("button", { name: "Use template docker" }).click();
  await page.getByRole("button", { name: "Use declared options" }).click();
  await expect(page.getByRole("button", { name: "Advanced settings" })).toHaveAttribute(
    "aria-expanded",
    "false",
  );
  await expect(page.getByLabel("Docker host", { exact: true })).toBeVisible();
  await expect(page.getByRole("region", { name: "Fleet input variables" })).toBeVisible();
  expect(
    await page.getByRole("dialog").evaluate((node) => node.scrollWidth <= node.clientWidth),
  ).toBe(true);
  await page.screenshot({
    path: "test-results/template-library-mobile.png",
    fullPage: true,
    animations: "disabled",
  });
  await page.getByRole("button", { name: "Advanced settings" }).click();
  await expect(page.getByLabel("Fleet input policy (JSON)")).toHaveValue(
    '{"runner_image":["runner:stable","runner:canary"]}',
  );
});

test("source switches retain drafts and discard late inspection responses", async ({ page }) => {
  await mockTemplateLibrary(page);
  let release: (() => void) | undefined;
  let requested = false;
  await page.route(
    `**/api/v1/template-artifacts/${encodeURIComponent(dockerDigest)}/variables`,
    async (route) => {
      requested = true;
      await new Promise<void>((resolve) => {
        release = resolve;
      });
      await route.fulfill({ json: variables });
    },
  );
  await page.goto("/templates");
  await page.getByRole("button", { name: "Use template docker" }).click();
  await expect.poll(() => requested).toBe(true);
  await page.getByRole("button", { name: "Advanced settings" }).click();
  await page.getByLabel("Bindings (JSON)").fill('{"docker_host":"my-draft"}');
  await page.getByLabel("Default template", { exact: true }).selectOption("kubernetes");
  await expect(page.getByRole("button", { name: "Inspect variables", exact: true })).toBeEnabled();
  release!();
  await expect(page.getByLabel("Docker host", { exact: true })).toHaveValue("my-draft");
  await expect(page.getByLabel("Bindings (JSON)")).toHaveValue('{"docker_host":"my-draft"}');
  await page.getByLabel("Profile key").fill("switched-source");
  let selectedDigest = "";
  await page.route("**/api/v1/template-profiles/switched-source", (route) => {
    selectedDigest = route.request().postDataJSON().artifact_digest;
    return route.fulfill({
      status: 202,
      json: { changeId: "switched", state: "Accepted", revision: 1 },
    });
  });
  await page.getByRole("button", { name: "Publish", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(selectedDigest).toBe(kubernetesDigest);
});

test("a successfully inspected archive uploads only once when published", async ({ page }) => {
  await mockTemplateLibrary(page);
  const bytes = Buffer.from("a-template-archive");
  const digest = `sha256:${createHash("sha256").update(bytes).digest("hex")}`;
  let uploads = 0;
  await page.route("**/api/v1/template-artifacts/*", (route) => {
    uploads++;
    expect(route.request().method()).toBe("PUT");
    expect(route.request().postDataBuffer()).toEqual(bytes);
    return route.fulfill({ status: 201, json: { digest } });
  });
  await page.route("**/api/v1/template-profiles/upload-inspected", (route) => {
    expect(route.request().postDataJSON().artifact_digest).toBe(digest);
    return route.fulfill({
      status: 202,
      json: { changeId: "uploaded", state: "Accepted", revision: 1 },
    });
  });
  await page.goto("/templates");
  await page.getByRole("button", { name: "Publish template", exact: true }).click();
  await page.getByLabel("Profile key").fill("upload-inspected");
  await page
    .getByLabel("Template archive (.tar.gz)")
    .setInputFiles({ name: "runner.tar.gz", mimeType: "application/gzip", buffer: bytes });
  await page.getByRole("button", { name: "Inspect variables", exact: true }).click();
  await expect(page.getByLabel("Docker host", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Publish", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(uploads).toBe(1);
});

test("read permission gates library and inspection without blocking manual publication", async ({
  page,
}) => {
  await mockApi(
    page,
    scopes.filter((scope) => scope !== "template.read"),
  );
  let discoveryReads = 0;
  await page.route(/\/api\/v1\/(template-sources|template-artifacts\/.*\/variables)/, (route) => {
    discoveryReads++;
    return route.fulfill({ status: 403, json: { code: "Forbidden", detail: "No permission" } });
  });
  await page.goto("/templates");
  await expect(page.getByRole("region", { name: "Default templates" })).toHaveCount(0);
  await page.getByRole("button", { name: "Publish template", exact: true }).click();
  await expect(page.getByRole("button", { name: "Inspect variables", exact: true })).toHaveCount(0);
  await expect(page.getByRole("dialog")).toContainText("Manual publishing is available");
  expect(discoveryReads).toBe(0);
});

test("unavailable and failed discovery preserve manual bindings", async ({ page }) => {
  await mockTemplateLibrary(page);
  let fail = false;
  await page.route("**/api/v1/template-artifacts/*/variables", (route) =>
    fail
      ? route.fulfill({
          status: 409,
          json: { code: "VariablesUnavailable", detail: "Unsupported Terraform expression" },
        })
      : route.fulfill({
          json: {
            artifactDigest: dockerDigest,
            available: false,
            reason: "Legacy untyped variables.",
            bindings: [],
            parameters: [],
          },
        }),
  );
  await page.goto("/templates");
  await page.getByRole("button", { name: "Use template docker" }).click();
  await expect(page.getByRole("dialog")).toContainText("Legacy untyped variables");
  await page.getByRole("button", { name: "Advanced settings" }).click();
  await page.getByLabel("Bindings (JSON)").fill('{"docker_host":"preserved"}');
  fail = true;
  await page.getByRole("button", { name: "Inspect variables", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("Unsupported Terraform expression");
  await expect(page.getByLabel("Bindings (JSON)")).toHaveValue('{"docker_host":"preserved"}');
  await expect(page.getByRole("button", { name: "Publish", exact: true })).toBeEnabled();
});

test("open form retains the exact library artifact when source metadata refreshes", async ({
  page,
}) => {
  await mockTemplateLibrary(page);
  await page.goto("/templates");
  await page.getByRole("button", { name: "Use template docker" }).click();
  await expect(page.getByLabel("Docker host", { exact: true })).toBeVisible();
  let refreshed = false;
  await page.route("**/api/v1/template-sources", (route) => {
    refreshed = true;
    return route.fulfill({
      json: { sources: [{ ...sources[0], artifactDigest: kubernetesDigest }] },
    });
  });
  await expect
    .poll(async () => {
      await page.evaluate(() => window.dispatchEvent(new Event("visibilitychange")));
      return refreshed;
    })
    .toBe(true);
  await page.getByLabel("Profile key").fill("pinned-library");
  let digest = "";
  await page.route("**/api/v1/template-profiles/pinned-library", (route) => {
    digest = route.request().postDataJSON().artifact_digest;
    return route.fulfill({
      status: 202,
      json: { changeId: "pinned", state: "Accepted", revision: 1 },
    });
  });
  await page.getByRole("button", { name: "Publish", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(digest).toBe(dockerDigest);
});
