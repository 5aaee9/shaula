import { expect, test, type Page } from "@playwright/test";
import { authProfile, fleet, mockApi, profile } from "./fixtures";

const target = {
  instance_url: "https://gitlab.example.test",
  scope: { kind: "group", path: "acme/build" },
};
const auth = {
  ...authProfile,
  key: "gitlab-auth",
  kind: "gitlab_token",
  schema_version: 1,
  active: {
    revision: 1,
    state: "Active",
    reason: null,
    schema_version: 1,
    gitlab: {
      target,
      validation: {
        server_version: "18.4.1",
        target_id: 42,
        checked_at_unix_ms: 1800000000000,
        valid_until_unix_ms: 1800000060000,
        runner_count: 0,
        token_expires_at: "2027-01-01",
      },
    },
  },
};
const forgejoAuth = { ...authProfile, key: "forgejo-auth", kind: "forgejo_token" };
const resource = {
  ...fleet("gitlab-build"),
  spec: {
    kind: "gitlab",
    gitlab: {
      ...target,
      auth_profile_ref: auth.key,
      runner_name_prefix: "shaula-gitlab-build-",
      tags: ["linux", "docker"],
      run_untagged: false,
    },
    capacity: { min_runners: 0, max_runners: 2 },
    template_profile_ref: "gitlab-docker",
    template_inputs: {},
  },
  resolved: {
    template: { ...fleet("x").resolved.template, key: "gitlab-docker" },
    authDesired: { profileKey: auth.key, revision: 1 },
  },
};

async function setup(page: Page) {
  await mockApi(page);
  await page.route("**/api/v1/github-auth-profiles", (route) =>
    route.fulfill({ json: { profiles: [authProfile, forgejoAuth, auth] } }),
  );
  await page.route("**/api/v1/github-auth-profiles/gitlab-auth", (route) =>
    route.fulfill({ headers: { etag: '"auth-inc:1"' }, json: auth }),
  );
  await page.route("**/api/v1/template-profiles", (route) =>
    route.fulfill({
      json: {
        profiles: [
          { ...profile("kubernetes-linux"), runnerBackend: "github" },
          { ...profile("forgejo-linux"), runnerBackend: "forgejo" },
          { ...profile("gitlab-docker"), runnerBackend: "gitlab" },
        ],
      },
    }),
  );
  await page.route("**/api/v1/fleets/gitlab-build", (route) =>
    route.fulfill({ headers: { etag: '"fleet-incarnation:2"' }, json: resource }),
  );
}

async function startGitlabFleet(page: Page, key: string) {
  await page.goto("/fleets/new");
  await page.getByLabel("Runner backend", { exact: true }).selectOption("gitlab");
  await page.getByLabel("Fleet key", { exact: true }).fill(key);
  const authentication = page.getByLabel("GitLab authentication profile", { exact: true });
  await expect(authentication.locator('option[value="github-build"]')).toHaveCount(0);
  await expect(authentication.locator('option[value="forgejo-auth"]')).toHaveCount(0);
  await authentication.selectOption("gitlab-auth");
  await expect(page.getByLabel("GitLab target")).toHaveText(
    "https://gitlab.example.test / acme/build (group)",
  );
  const templates = page.getByLabel("Template profile", { exact: true });
  await expect(templates.locator('option[value="kubernetes-linux"]')).toHaveCount(0);
  await expect(templates.locator('option[value="forgejo-linux"]')).toHaveCount(0);
  await templates.selectOption("gitlab-docker");
  await page.getByRole("button", { name: "Load latest Active" }).click();
}

async function captureFleetPut(page: Page, key: string) {
  const sent: { body?: Record<string, unknown> } = {};
  await page.route(`**/api/v1/fleets/${key}`, (route) => {
    sent.body = route.request().postDataJSON();
    return route.fulfill({
      status: 202,
      json: { changeId: "change", revision: 1, state: "Pending" },
    });
  });
  return sent;
}

test("GitLab fleet details render tags and run-single semantics without GitHub fields", async ({
  page,
}) => {
  await setup(page);
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto("/fleets/gitlab-build");
  await expect(page.getByRole("heading", { name: "gitlab-build" })).toBeVisible();
  await expect(page.getByText("GitLab target", { exact: true })).toBeVisible();
  await expect(page.getByText("Waiting jobs", { exact: true })).toBeVisible();
  await expect(page.getByText("Runner group", { exact: true })).toHaveCount(0);
  await expect(page.getByText("docker", { exact: true })).toBeVisible();
  await expect(page.getByText(/image: is ignored/)).toBeVisible();
  await page.screenshot({ path: "test-results/gitlab-fleet-detail.png", fullPage: true });
  await page.getByRole("button", { name: "Edit fleet", exact: true }).click();
  await expect(page.getByLabel("Tags", { exact: true })).toHaveValue("linux, docker");
  await expect(page.getByLabel("Tags", { exact: true })).toBeDisabled();
  await expect(page.getByLabel("GitLab authentication profile", { exact: true })).toBeDisabled();
  expect(errors).toEqual([]);
});

test("GitHub fleet creation does not offer GitLab authentication profiles", async ({ page }) => {
  await setup(page);
  await page.goto("/fleets/new");
  const authentication = page.getByLabel("GitHub authentication profile", { exact: true });
  await expect(authentication.locator('option[value="github-build"]')).toHaveCount(1);
  await expect(authentication.locator('option[value="gitlab-auth"]')).toHaveCount(0);
  await expect(authentication.locator('option[value="forgejo-auth"]')).toHaveCount(0);
});

test("GitLab creation submits the profile target, tags and no GitHub fields", async ({ page }) => {
  await setup(page);
  await startGitlabFleet(page, "gitlab-new");
  await page.getByLabel("Tags", { exact: true }).fill("linux, docker");
  const sent = await captureFleetPut(page, "gitlab-new");
  await page.getByRole("button", { name: "Create fleet", exact: true }).click();
  await expect
    .poll(() => sent.body)
    .toEqual({
      kind: "gitlab",
      gitlab: {
        ...target,
        auth_profile_ref: auth.key,
        runner_name_prefix: "shaula-gitlab-new-",
        tags: ["linux", "docker"],
        run_untagged: false,
      },
      capacity: { min_runners: 0, max_runners: 10 },
      template_profile_ref: "gitlab-docker",
      template_inputs: {},
    });
});

test("GitLab untagged fleets may omit tags; tagless tagged fleets are rejected", async ({
  page,
}) => {
  await setup(page);
  await startGitlabFleet(page, "gitlab-untagged");
  const sent = await captureFleetPut(page, "gitlab-untagged");
  // Separators alone satisfy the required field but declare no tag.
  await page.getByLabel("Tags", { exact: true }).fill(" , ");
  await page.getByRole("button", { name: "Create fleet", exact: true }).click();
  await expect(page.getByRole("alert").last()).toContainText("at least one runner tag");
  expect(sent.body).toBeUndefined();
  await page.getByLabel("Runner name prefix", { exact: true }).fill("linux-");
  await page.getByLabel("Run untagged jobs", { exact: true }).check();
  await page.getByRole("button", { name: "Create fleet", exact: true }).click();
  await expect
    .poll(() => sent.body?.gitlab)
    .toMatchObject({
      runner_name_prefix: "linux-",
      tags: [],
      run_untagged: true,
    });
});

for (const scope of ["group", "project"] as const) {
  test(`GitLab token publication uses the ${scope} scope and write-only credential`, async ({
    page,
  }) => {
    await setup(page);
    await page.goto("/auth/new");
    await page.getByLabel("Runner backend", { exact: true }).selectOption("gitlab");
    await page.getByLabel("Profile key", { exact: true }).fill("new-gitlab");
    await page.getByLabel("GitLab instance URL", { exact: true }).fill(target.instance_url);
    await page.getByLabel("GitLab scope", { exact: true }).selectOption(scope);
    const path = scope === "project" ? "acme/build/app" : "acme/build";
    await page.getByLabel(scope === "project" ? "Project path" : "Group path").fill(path);
    await page.getByLabel("Access token", { exact: true }).fill("test-only-canary");
    await expect(page.getByLabel("Access token", { exact: true })).toHaveAttribute(
      "type",
      "password",
    );
    let body: Record<string, unknown> | undefined;
    await page.route("**/api/v1/github-auth-profiles/new-gitlab", (route) => {
      body = route.request().postDataJSON();
      return route.fulfill({
        status: 202,
        json: { changeId: "change", revision: 1, state: "Pending" },
      });
    });
    await page.getByRole("button", { name: "Create profile", exact: true }).click();
    await expect
      .poll(() => body)
      .toEqual({
        kind: "gitlab_token",
        instance_url: target.instance_url,
        scope: { kind: scope, path },
        token: "test-only-canary",
      });
  });
}

test("GitLab token rotation shows validation, locks the target and keeps the scope", async ({
  page,
}) => {
  await setup(page);
  await page.goto("/auth?key=gitlab-auth");
  await expect(page.getByRole("heading", { name: "Active GitLab target" })).toBeVisible();
  await expect(page.getByLabel("GitLab validation")).toContainText("2027-01-01");
  await expect(page.getByText("GitLab token", { exact: true }).first()).toBeVisible();
  await expect(page.getByRole("button", { name: "Edit target policy" })).toHaveCount(0);
  await page.screenshot({ path: "test-results/gitlab-auth-detail.png", fullPage: true });
  await page.getByRole("button", { name: "Rotate credential" }).click();
  await expect(page.getByLabel("GitLab instance URL", { exact: true })).toBeDisabled();
  await expect(page.getByLabel("GitLab scope", { exact: true })).toBeDisabled();
  await expect(page.getByLabel("Group path", { exact: true })).toHaveValue("acme/build");
  await page.getByLabel("Access token", { exact: true }).fill("replacement-canary");
  let body: unknown;
  await page.route("**/api/v1/github-auth-profiles/gitlab-auth", (route) => {
    if (route.request().method() !== "PUT")
      return route.fulfill({ headers: { etag: '"auth-inc:1"' }, json: auth });
    expect(route.request().headers()["if-match"]).toBe('"auth-inc:1"');
    body = route.request().postDataJSON();
    return route.fulfill({
      status: 202,
      json: { changeId: "change", revision: 2, state: "Pending" },
    });
  });
  await page.getByRole("button", { name: "Rotate credential", exact: true }).click();
  await expect
    .poll(() => body)
    .toEqual({ kind: "gitlab_token", ...target, token: "replacement-canary" });
});

test("GitLab fields fit desktop and mobile with visible labels", async ({ page }) => {
  await setup(page);
  await page.goto("/fleets/new");
  await page.getByLabel("Runner backend", { exact: true }).selectOption("gitlab");
  await page
    .getByLabel("GitLab authentication profile", { exact: true })
    .selectOption("gitlab-auth");
  for (const [name, width] of [
    ["desktop", 1280],
    ["mobile", 390],
  ] as const) {
    await page.setViewportSize({ width, height: 900 });
    await expect(page.getByLabel("Tags", { exact: true })).toBeVisible();
    await expect(page.getByLabel("Run untagged jobs", { exact: true })).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
      true,
    );
    await page.screenshot({ path: `test-results/gitlab-management-${name}.png`, fullPage: true });
  }
});
