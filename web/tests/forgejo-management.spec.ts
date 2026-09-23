import { expect, test, type Page } from "@playwright/test";
import { authProfile, fleet, mockApi, profile } from "./fixtures";

const target = {
  instance_url: "https://forgejo.example.test",
  scope: { kind: "repository", owner: "acme", name: "build" },
};
const auth = {
  ...authProfile,
  key: "forgejo-auth",
  kind: "forgejo_token",
  schema_version: 1,
  active: {
    revision: 1,
    state: "Active",
    reason: null,
    schema_version: 1,
    forgejo: {
      target,
      validation: {
        server_version: "16.0.4",
        principal_id: 1,
        target_id: 2,
        checked_at_unix_ms: 1800000000000,
        valid_until_unix_ms: 1800000060000,
        runner_count: 0,
      },
    },
  },
};
const resource = {
  ...fleet("forgejo-build"),
  spec: {
    kind: "forgejo",
    forgejo: {
      ...target,
      auth_profile_ref: auth.key,
      runner_name_prefix: "shaula-forgejo-build-",
      labels: ["linux:host"],
    },
    capacity: { min_runners: 0, max_runners: 2 },
    template_profile_ref: "forgejo-linux",
    template_inputs: {},
  },
  resolved: {
    template: { ...fleet("x").resolved.template, key: "forgejo-linux" },
    authDesired: { profileKey: auth.key, revision: 1 },
  },
};

async function setup(page: Page) {
  await mockApi(page);
  await page.route("**/api/v1/github-auth-profiles", (route) =>
    route.fulfill({ json: { profiles: [authProfile, auth] } }),
  );
  await page.route("**/api/v1/github-auth-profiles/forgejo-auth", (route) =>
    route.fulfill({ headers: { etag: '"auth-inc:1"' }, json: auth }),
  );
  await page.route("**/api/v1/template-profiles", (route) =>
    route.fulfill({
      json: {
        profiles: [
          { ...profile("kubernetes-linux"), runnerBackend: "github" },
          { ...profile("forgejo-linux"), runnerBackend: "forgejo" },
          { ...profile("unresolved"), runnerBackend: null },
        ],
      },
    }),
  );
  await page.route("**/api/v1/fleets/forgejo-build", (route) =>
    route.fulfill({ headers: { etag: '"fleet-incarnation:2"' }, json: resource }),
  );
}

test("mixed fleets and Forgejo details render without GitHub fields", async ({ page }) => {
  await setup(page);
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.route("**/api/v1/fleets", (route) =>
    route.fulfill({
      json: {
        fleets: [
          { key: "linux-build", revision: 2, incarnation: "fleet-incarnation" },
          { key: "forgejo-build", revision: 2, incarnation: "fleet-incarnation" },
        ],
      },
    }),
  );
  await page.goto("/fleets");
  await expect(page.getByText("forgejo-build", { exact: true })).toBeVisible();
  await expect(page.getByText("linux-build", { exact: true })).toBeVisible();
  await page.goto("/fleets/forgejo-build");
  await expect(page.getByRole("heading", { name: "forgejo-build" })).toBeVisible();
  await expect(page.getByText("Waiting jobs", { exact: true })).toBeVisible();
  await expect(page.getByText("Runner group", { exact: true })).toHaveCount(0);
  await page.getByRole("button", { name: "Edit fleet", exact: true }).click();
  await expect(page.getByLabel("Labels", { exact: true })).toBeDisabled();
  await expect(page.getByLabel("Forgejo authentication profile", { exact: true })).toBeDisabled();
  expect(errors).toEqual([]);
});

test("Forgejo creation only selects compatible templates and submits no GitHub or pool fields", async ({
  page,
}) => {
  await setup(page);
  await page.goto("/fleets/new");
  await page.getByLabel("Runner backend", { exact: true }).selectOption("forgejo");
  await page.getByLabel("Fleet key", { exact: true }).fill("forgejo-new");
  const authentication = page.getByLabel("Forgejo authentication profile", { exact: true });
  await expect(authentication.locator('option[value="github-build"]')).toHaveCount(0);
  await authentication.selectOption("forgejo-auth");
  await page.getByLabel("Labels", { exact: true }).fill("linux:host");
  const templates = page.getByLabel("Template profile", { exact: true });
  await expect(templates.locator('option[value="kubernetes-linux"]')).toHaveCount(0);
  await expect(templates.locator('option[value="unresolved"]')).toHaveCount(0);
  await expect(page.getByLabel("Template placement", { exact: true })).toHaveValue("single");
  await templates.selectOption("forgejo-linux");
  await page.getByRole("button", { name: "Load latest Active" }).click();
  let body: Record<string, unknown> | undefined;
  await page.route("**/api/v1/fleets/forgejo-new", (route) => {
    body = route.request().postDataJSON();
    expect(route.request().headers()["if-none-match"]).toBe("*");
    return route.fulfill({
      status: 202,
      json: { changeId: "change", revision: 1, state: "Pending" },
    });
  });
  await page.getByRole("button", { name: "Create fleet", exact: true }).click();
  await expect
    .poll(() => body)
    .toEqual({
      kind: "forgejo",
      forgejo: {
        ...target,
        auth_profile_ref: auth.key,
        runner_name_prefix: "shaula-forgejo-new-",
        labels: ["linux:host"],
      },
      capacity: { min_runners: 0, max_runners: 10 },
      template_profile_ref: "forgejo-linux",
      template_inputs: {},
    });
});

for (const placement of ["pool", "shared"]) {
  test(`Forgejo ${placement} placement submits exactly one routing source`, async ({ page }) => {
    await setup(page);
    await page.route("**/api/v1/template-pools", (route) =>
      route.fulfill({
        json: { pools: [{ key: "forgejo-pool", revision: 1, incarnation: "pool-inc" }] },
      }),
    );
    await page.goto("/fleets/new");
    await page.getByLabel("Runner backend", { exact: true }).selectOption("forgejo");
    await page.getByLabel("Fleet key", { exact: true }).fill("forgejo-weighted");
    await page
      .getByLabel("Forgejo authentication profile", { exact: true })
      .selectOption("forgejo-auth");
    await page.getByLabel("Labels", { exact: true }).fill("linux:host");
    await page.getByLabel("Template placement", { exact: true }).selectOption(placement);
    if (placement === "pool") {
      const templates = page.getByLabel("Template profile", { exact: true });
      await expect(templates.locator('option[value="kubernetes-linux"]')).toHaveCount(0);
      await templates.selectOption("forgejo-linux");
      await page.getByLabel("Weight", { exact: true }).fill("3");
    } else {
      await page.getByLabel("Template pool", { exact: true }).selectOption("forgejo-pool");
    }
    for (const width of [1280, 390]) {
      await page.setViewportSize({ width, height: 900 });
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
        true,
      );
      await page.screenshot({
        path: `test-results/forgejo-${placement}-${width}.png`,
        fullPage: true,
      });
    }
    let body: Record<string, unknown> | undefined;
    await page.route("**/api/v1/fleets/forgejo-weighted", (route) => {
      body = route.request().postDataJSON();
      return route.fulfill({
        status: 202,
        json: { changeId: "change", revision: 1, state: "Pending" },
      });
    });
    await page.getByRole("button", { name: "Create fleet", exact: true }).click();
    await expect.poll(() => body?.kind).toBe("forgejo");
    expect(body).not.toHaveProperty("github");
    expect(body).not.toHaveProperty("template_profile_ref");
    expect(body).not.toHaveProperty("template_inputs");
    if (placement === "pool") {
      expect(body).not.toHaveProperty("template_pool_ref");
      expect(body?.template_pool).toEqual({
        failure_policy: "backpressure",
        members: [
          {
            key: "member-1",
            template_profile_ref: "forgejo-linux",
            weight: 3,
            template_inputs: {},
          },
        ],
      });
    } else {
      expect(body).not.toHaveProperty("template_pool");
      expect(body?.template_pool_ref).toBe("forgejo-pool");
    }
    for (const width of [1280, 390]) {
      await page.setViewportSize({ width, height: 900 });
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
        true,
      );
    }
  });
}

test("Forgejo shared-pool edit retains the frozen route without loading a single-template contract", async ({
  page,
}) => {
  await setup(page);
  const pooled = {
    ...resource,
    spec: {
      ...resource.spec,
      template_profile_ref: undefined,
      template_inputs: undefined,
      template_pool_ref: "forgejo-pool",
    },
    resolved: { ...resource.resolved, template: null },
  };
  let body: Record<string, unknown> | undefined;
  let contractReads = 0;
  await page.route("**/api/v1/template-profiles/**/input-contract", (route) => {
    contractReads++;
    return route.fallback();
  });
  await page.route("**/api/v1/template-pools", (route) => route.fulfill({ json: { pools: [] } }));
  await page.route("**/api/v1/fleets/forgejo-build", (route) => {
    if (route.request().method() === "PUT") {
      expect(route.request().headers()["if-match"]).toBe('"fleet-incarnation:2"');
      body = route.request().postDataJSON();
      return route.fulfill({
        status: 202,
        json: { changeId: "change", revision: 3, state: "Pending" },
      });
    }
    return route.fulfill({ headers: { etag: '"fleet-incarnation:2"' }, json: pooled });
  });
  await page.goto("/fleets/forgejo-build/edit");
  await expect(page.getByLabel("Template placement", { exact: true })).toHaveValue("shared");
  await expect(page.getByLabel("Template placement", { exact: true })).toBeDisabled();
  await page.getByLabel("Maximum runners", { exact: true }).fill("3");
  await page.getByRole("button", { name: "Save changes", exact: true }).click();
  await expect.poll(() => body?.template_pool_ref).toBe("forgejo-pool");
  expect(body).not.toHaveProperty("template_profile_ref");
  expect(body).not.toHaveProperty("template_inputs");
  expect(contractReads).toBe(0);
});

for (const scope of ["instance", "user", "organization", "repository"]) {
  test(`Forgejo token publication uses the ${scope} scope and write-only credential`, async ({
    page,
  }) => {
    await setup(page);
    await page.goto("/auth/new");
    await page.getByLabel("Runner backend", { exact: true }).selectOption("forgejo");
    await page.getByLabel("Profile key", { exact: true }).fill("new-forgejo");
    await page.getByLabel("Forgejo instance URL", { exact: true }).fill(target.instance_url);
    await page.getByLabel("Forgejo scope", { exact: true }).selectOption(scope);
    if (scope === "organization")
      await page.getByLabel("Organization", { exact: true }).fill("acme");
    if (scope === "repository") {
      await page.getByLabel("Repository owner", { exact: true }).fill("acme");
      await page.getByLabel("Repository", { exact: true }).fill("build");
    }
    await page.getByLabel("Access token", { exact: true }).fill("test-only-canary");
    await expect(page.getByLabel("Access token", { exact: true })).toHaveAttribute(
      "type",
      "password",
    );
    let body: Record<string, unknown> | undefined;
    await page.route("**/api/v1/github-auth-profiles/new-forgejo", (route) => {
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
        kind: "forgejo_token",
        instance_url: target.instance_url,
        scope:
          scope === "repository"
            ? target.scope
            : scope === "organization"
              ? { kind: scope, name: "acme" }
              : { kind: scope },
        token: "test-only-canary",
      });
  });
}

test("Forgejo token rotation locks scope, previews impact, and uses the reviewed version", async ({
  page,
}) => {
  await setup(page);
  await page.goto("/auth?key=forgejo-auth");
  await expect(page.getByRole("heading", { name: "Active Forgejo target" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Edit target policy" })).toHaveCount(0);
  await page.getByRole("button", { name: "Rotate credential" }).click();
  await expect(page.getByLabel("Forgejo instance URL", { exact: true })).toBeDisabled();
  await expect(page.getByLabel("Forgejo scope", { exact: true })).toBeDisabled();
  await expect(page.getByLabel("Access token", { exact: true })).toHaveValue("");
  await expect(page.getByLabel("Credential rotation impact")).toContainText(
    "not a seamless handoff",
  );
  await page.getByLabel("Access token", { exact: true }).fill("replacement-canary");
  let body: unknown;
  await page.route("**/api/v1/github-auth-profiles/forgejo-auth", (route) => {
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
    .toEqual({ kind: "forgejo_token", ...target, token: "replacement-canary" });
});

for (const [key, secretLabel] of [
  ["github-build", "Private key (PEM)"],
  ["forgejo-auth", "Access token"],
]) {
  test(`${key} rotation blocks malformed impact and resumes after recovery`, async ({ page }) => {
    await setup(page);
    let recovered = false;
    let writes = 0;
    await page.route(`**/api/v1/github-auth-profiles/${key}/impact`, (route) =>
      route.fulfill({ json: recovered ? { liveFleets: [] } : {} }),
    );
    await page.route(`**/api/v1/github-auth-profiles/${key}`, (route) => {
      if (route.request().method() !== "PUT") return route.fallback();
      writes++;
      return route.fulfill({
        status: 202,
        json: { changeId: "rotation", revision: 2, state: "Pending" },
      });
    });
    await page.goto(`/auth/${key}/rotate`);
    await page.getByLabel(secretLabel, { exact: true }).fill("replacement-canary");
    const submit = page.getByRole("button", { name: "Rotate credential", exact: true });
    await expect(page.getByRole("alert").first()).toContainText(/impact.*unavailable/i);
    await expect(submit).toBeDisabled();
    await page
      .getByRole("form", { name: "Authentication configuration" })
      .evaluate((form: HTMLFormElement) => form.requestSubmit());
    await expect(page.getByRole("alert").last()).toContainText(/impact/i);
    expect(writes).toBe(0);
    recovered = true;
    await expect(submit).toBeEnabled({ timeout: 15_000 });
    await expect(page.getByLabel(secretLabel, { exact: true })).toHaveValue("replacement-canary");
    await submit.click();
    await expect.poll(() => writes).toBe(1);
  });
}

test("Forgejo fields fit desktop and mobile with visible labels", async ({ page }) => {
  await setup(page);
  await page.goto("/fleets/new");
  await page.getByLabel("Runner backend", { exact: true }).selectOption("forgejo");
  await page
    .getByLabel("Forgejo authentication profile", { exact: true })
    .selectOption("forgejo-auth");
  for (const [name, width] of [
    ["desktop", 1280],
    ["mobile", 390],
  ] as const) {
    await page.setViewportSize({ width, height: 900 });
    await expect(page.getByLabel("Labels", { exact: true })).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
      true,
    );
    await page.screenshot({ path: `test-results/forgejo-management-${name}.png`, fullPage: true });
  }
});
