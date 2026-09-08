import { expect, test } from "@playwright/test";
import { mockApi } from "./fixtures";
import { LEGACY_APP_PROFILE, V2_PROFILE } from "./auth-fixtures";

test("per-account observations expire locally and rejected candidate status stays separate", async ({
  page,
}) => {
  const now = Date.now();
  await page.clock.install({ time: new Date(now) });
  await mockApi(page);
  await page.route("**/api/v1/github-auth-profiles/shared-github", (route) =>
    route.fulfill({
      headers: { etag: '"auth-inc:2"' },
      json: {
        ...V2_PROFILE,
        active: {
          ...V2_PROFILE.active,
          bindings: [
            {
              ...V2_PROFILE.active.bindings[0],
              health: "Healthy",
              reason: "CurrentFleetAccessVerified",
              checked_at_ms: now,
              valid_until_ms: now + 60_000,
            },
            {
              ...V2_PROFILE.active.bindings[0],
              account_id: 200,
              login: "5aaee9",
              account_kind: "user",
              installation_id: 22,
              health: "Blocked",
              reason: "PermissionDenied",
              checked_at_ms: now,
              valid_until_ms: now + 15_000,
              affected_fleets: ["personal-repo"],
            },
          ],
        },
        desired: { ...V2_PROFILE.desired, state: "Rejected", reason: "AuthIdentityMismatch" },
      },
    }),
  );
  await page.goto("/auth?key=shared-github");
  await expect(page.getByText("Candidate r2: Rejected")).toBeVisible();
  const org = page.getByRole("row").filter({ hasText: "Indexyz" });
  const user = page.getByRole("row").filter({ hasText: "5aaee9" });
  await expect(org.getByText("Healthy", { exact: true })).toBeVisible();
  await expect(user.getByText("Blocked", { exact: true })).toBeVisible();
  await expect(user.getByRole("link", { name: "personal-repo" })).toHaveAttribute(
    "href",
    "/fleets/personal-repo",
  );
  await page.clock.fastForward(15_001);
  await expect(user.getByText("Unknown", { exact: true })).toBeVisible();
  await expect(org.getByText("Healthy", { exact: true })).toBeVisible();
  await page.clock.fastForward(45_000);
  await expect(org.getByText("Unknown", { exact: true })).toBeVisible();
});

test("typed v2 preview distinguishes org runners from the same org repositories and includes blocked Fleets", async ({
  page,
}) => {
  await mockApi(page);
  await page.route("**/api/v1/github-auth-profiles/shared-github", (route) =>
    route.fulfill({ headers: { etag: '"auth-inc:2"' }, json: V2_PROFILE }),
  );
  await page.route("**/api/v1/github-auth-profiles/shared-github/impact", (route) =>
    route.fulfill({
      json: {
        desiredRevision: 2,
        liveFleets: [
          {
            fleetKey: "org-runners",
            phase: "Blocked",
            target: { kind: "organization", owner: "Indexyz" },
          },
          {
            fleetKey: "personal-repo",
            phase: "Ready",
            target: { kind: "repository", owner: "5aaee9", repository: "future" },
          },
          { fleetKey: "unknown-target", phase: "Ready", target: null },
        ],
      },
    }),
  );
  await page.goto("/auth?key=shared-github");
  await page.getByRole("button", { name: "Rotate credential" }).click();
  await page.getByLabel("Selector type").first().selectOption("account_repositories");
  await page.getByLabel("Account type").first().selectOption("organization");
  const preview = page.getByRole("region", { name: "Policy change preview" });
  await expect(preview.getByText("Indexyz (org runners)", { exact: true })).toBeVisible();
  await expect(
    preview.getByText("Indexyz (organization repositories)", { exact: true }),
  ).toBeVisible();
  await expect(preview.getByText(/org-runners.*Blocks activation: target removed/)).toBeVisible();
  await expect(preview.getByText(/personal-repo.*Remains covered/)).toBeVisible();
  await expect(preview.getByText(/unknown-target.*Coverage unknown/)).toBeVisible();
  await expect(page.getByRole("button", { name: "Publish policy" })).toBeEnabled();
});

test("legacy upgrade previews retained actual targets and refreshes impact without discarding form edits", async ({
  page,
}) => {
  await mockApi(page);
  await page.route("**/api/v1/github-auth-profiles/legacy-app", (route) =>
    route.fulfill({ headers: { etag: '"auth-inc:1"' }, json: LEGACY_APP_PROFILE }),
  );
  let calls = 0;
  await page.route("**/api/v1/github-auth-profiles/legacy-app/impact", (route) => {
    calls += 1;
    return route.fulfill({
      json: {
        desiredRevision: 1,
        liveFleets: [
          {
            fleetKey: calls > 1 ? "new-dependent" : "retained-dependent",
            phase: "Decommissioning",
            target: { kind: "repository", owner: "acme", repository: "build-tools" },
          },
        ],
      },
    });
  });
  await page.goto("/auth?key=legacy-app");
  await page.getByRole("button", { name: "Rotate credential" }).click();
  await page.getByLabel("Upgrade to multi-account policy").check();
  const preview = page.getByRole("region", { name: "Policy change preview" });
  await expect(preview.getByText(/retained-dependent.*Remains covered/)).toBeVisible();
  await page.getByLabel("App ID").fill("4863460");
  await page.getByRole("button", { name: "Add selector" }).click();
  await page.getByLabel("Owner").last().fill("other-org");
  await expect(preview.getByText(/new-dependent.*Remains covered/)).toBeVisible({ timeout: 8_000 });
  await expect(page.getByLabel("App ID")).toHaveValue("4863460");
  await expect(page.getByLabel("Owner").last()).toHaveValue("other-org");
  await expect(preview.getByText("other-org (org runners)", { exact: true })).toBeVisible();
});

test("impact read failure is explicit and cannot be presented as no live dependencies", async ({
  page,
}) => {
  await mockApi(page);
  await page.route("**/api/v1/github-auth-profiles/shared-github", (route) =>
    route.fulfill({ headers: { etag: '"auth-inc:2"' }, json: V2_PROFILE }),
  );
  await page.route("**/api/v1/github-auth-profiles/shared-github/impact", (route) =>
    route.fulfill({ status: 500, json: { code: "Internal", detail: "Unavailable" } }),
  );
  await page.goto("/auth?key=shared-github");
  await page.getByRole("button", { name: "Rotate credential" }).click();
  await expect(
    page.getByRole("dialog").getByRole("button", { name: "Rotate credential" }),
  ).toBeDisabled();
  await expect(page.getByText("No live Fleets reference this profile.")).toHaveCount(0);
  await expect(page.getByText(/Fleet impact is unavailable/)).toBeVisible({ timeout: 10_000 });
});

test("rejected first upgrade keeps active legacy identity and permits correcting numeric App ID", async ({
  page,
}) => {
  await mockApi(page);
  await page.route("**/api/v1/github-auth-profiles/legacy-app", (route) =>
    route.fulfill({
      headers: { etag: '"auth-inc:2"' },
      json: {
        ...LEGACY_APP_PROFILE,
        desiredRevision: 2,
        schema_version: 2,
        app_id: "999",
        active: {
          revision: 1,
          schema_version: 1,
          state: "Active",
          reason: null,
          identity: LEGACY_APP_PROFILE.identity,
          target_allowlist: LEGACY_APP_PROFILE.target_allowlist,
        },
        desired: {
          revision: 2,
          schema_version: 2,
          state: "Rejected",
          reason: "AuthIdentityMismatch",
          app_id: "999",
          target_policy: [{ kind: "organization", owner: "acme" }],
          bindings: [],
        },
      },
    }),
  );
  await page.goto("/auth?key=legacy-app");
  await expect(page.getByText("Candidate r2: Rejected")).toBeVisible();
  await expect(page.getByText("AuthIdentityMismatch")).toBeVisible();
  await expect(page.getByText(LEGACY_APP_PROFILE.identity)).toBeVisible();
  await page.getByRole("button", { name: "Rotate credential" }).click();
  await expect(page.getByLabel("App ID")).toBeEnabled();
  await page.getByLabel("App ID").fill("4863460");
  await page.getByLabel("Upgrade to multi-account policy").uncheck();
  await expect(page.getByLabel("App ID")).toHaveValue("Iv23legacy");
  await expect(page.getByLabel("Installation ID")).toHaveValue("34");
});

test("Fleet details show exact desired and observed route identities with separate numeric pins", async ({
  page,
}) => {
  await mockApi(page);
  const reference = { profileKey: "shared-github", revision: 2 };
  const route = {
    ...reference,
    githubHost: "github.com",
    appId: "4863460",
    account: { login: "5aaee9", id: 10, kind: "user" },
    installationId: 20,
    target: { kind: "repository", owner: "5aaee9", repository: "repo" },
    organizationId: null,
    repositoryId: 30,
    repositoryOwnerId: 10,
  };
  await page.route("**/api/v1/fleets/linux-build/status", (request) =>
    request.fulfill({
      json: {
        fleetKey: "linux-build",
        desiredRevision: 2,
        observedRevision: 2,
        phase: "Blocked",
        conditions: [],
        capacity: { assignedDemand: 0, target: 0, effective: 0, occupancy: 0 },
        lastError: null,
        githubAuth: {
          desired: reference,
          observed: { ...reference, revision: 1 },
          handoffState: "Blocked",
          context: {
            desired: reference,
            observed: { ...reference, revision: 1 },
            state: "Blocked",
            reason: "InstallationSuspended",
            desiredRoute: route,
            observedRoute: { ...route, revision: 1, installationId: 19 },
          },
        },
      },
    }),
  );
  await page.goto("/fleets/linux-build");
  const desired = page.getByLabel("Desired route", { exact: true });
  const observed = page.getByLabel("Observed route", { exact: true });
  await expect(desired.getByText("shared-github / r2")).toBeVisible();
  await expect(desired.getByText(/installation #20/)).toBeVisible();
  await expect(observed.getByText(/installation #19/)).toBeVisible();
  await expect(desired.getByText("Repository #30 · owner #10")).toBeVisible();
  await expect(desired.getByText("github.com · App #4863460")).toBeVisible();
});
