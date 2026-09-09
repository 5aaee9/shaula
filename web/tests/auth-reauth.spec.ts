import { expect, test, type Page, type Route } from "@playwright/test";
import { UNSUPPORTED_PROFILE, V2_PROFILE } from "./auth-fixtures";
import { mockApi, scopes } from "./fixtures";

const PROFILE_URL = "/auth?key=shared-github";
const LINK_PATH = "**/api/v1/github-auth-profiles/shared-github/installation-link";
const INSTALL_URL = "https://github.com/apps/shaula-runners/installations/new";
const INSTALL_LINK = {
  url: INSTALL_URL,
  appId: V2_PROFILE.active.app_id,
  revision: V2_PROFILE.activeRevision,
  incarnation: V2_PROFILE.incarnation,
};

async function mockProfile(page: Page, profile: object = V2_PROFILE, permissions = scopes) {
  await mockApi(page, permissions);
  await page.route("**/api/v1/github-auth-profiles/shared-github", (route) =>
    route.fulfill({ headers: { etag: '"auth-inc:2"' }, json: profile }),
  );
}

async function mockGitHub(page: Page) {
  const requests: string[] = [];
  await page.route("https://github.com/**", (route) => {
    requests.push(route.request().url());
    return route.fulfill({
      contentType: "text/html",
      body: "<h1>Install Shaula runners</h1>",
    });
  });
  return requests;
}

test("Re-auth looks up the active App only on click without publishing or asking for secrets", async ({
  page,
}) => {
  await mockProfile(page, {
    ...V2_PROFILE,
    app_id: "9999999",
    desired: { ...V2_PROFILE.desired, app_id: "9999999" },
  });
  const githubRequests = await mockGitHub(page);
  const profileRequests: { method: string; body: string | null }[] = [];
  page.on("request", (request) => {
    if (request.url().includes("/github-auth-profiles")) {
      profileRequests.push({ method: request.method(), body: request.postData() });
    }
  });
  let linkRequests = 0;
  await page.route(LINK_PATH, (route) => {
    linkRequests++;
    return route.fulfill({ json: INSTALL_LINK });
  });
  await page.goto(PROFILE_URL);
  const reauth = page.getByRole("button", { name: "Re-auth", exact: true });
  await expect(reauth).toBeEnabled();
  await expect(page.getByLabel("Private key (PEM)", { exact: true })).toHaveCount(0);
  expect(linkRequests).toBe(0);
  expect(githubRequests).toEqual([]);
  await reauth.click();
  await expect(page).toHaveURL(INSTALL_URL);
  await expect(page.getByRole("heading", { name: "Install Shaula runners" })).toBeVisible();
  expect(linkRequests).toBe(1);
  expect(githubRequests).toEqual([INSTALL_URL]);
  expect(
    profileRequests.every((request) => request.method === "GET" && request.body === null),
  ).toBe(true);
});

for (const permission of ["auth.read", "auth.write"]) {
  test(`Re-auth is unavailable without ${permission}`, async ({ page }) => {
    await mockProfile(
      page,
      V2_PROFILE,
      scopes.filter((scope) => scope !== permission),
    );
    let linkRequests = 0;
    await page.route(LINK_PATH, (route) => {
      linkRequests++;
      return route.fulfill({ json: INSTALL_LINK });
    });
    await page.goto(PROFILE_URL);
    await expect(page.getByRole("heading", { name: "Authentication", exact: true })).toBeVisible();
    if (permission === "auth.read") {
      await expect(
        page.getByText("Authentication profile read permission is required."),
      ).toBeVisible();
    } else {
      await expect(page.getByRole("heading", { name: "shared-github", exact: true })).toBeVisible();
    }
    await expect(page.getByRole("button", { name: "Re-auth", exact: true })).toHaveCount(0);
    expect(linkRequests).toBe(0);
  });
}

const unavailableProfiles = [
  ["retiring", { ...V2_PROFILE, status: "Retiring" }],
  ["retired", { ...V2_PROFILE, status: "Retired" }],
  ["unsupported profile", { ...UNSUPPORTED_PROFILE, key: V2_PROFILE.key }],
  [
    "unsupported active revision",
    { ...V2_PROFILE, active: { ...V2_PROFILE.active, schema_version: 1 } },
  ],
  ["personal access token", { ...V2_PROFILE, kind: "pat" }],
  ["missing credential", { ...V2_PROFILE, credential_present: false }],
  ["no active revision", { ...V2_PROFILE, activeRevision: null, active: undefined }],
  ["missing active revision detail", { ...V2_PROFILE, active: undefined }],
] as const;

for (const [description, profile] of unavailableProfiles) {
  test(`Re-auth is unavailable for ${description}`, async ({ page }) => {
    await mockProfile(page, profile);
    let linkRequests = 0;
    await page.route(LINK_PATH, (route) => {
      linkRequests++;
      return route.fulfill({ json: INSTALL_LINK });
    });
    await page.goto(PROFILE_URL);
    await expect(page.getByRole("heading", { name: "shared-github", exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: "Re-auth", exact: true })).toHaveCount(0);
    expect(linkRequests).toBe(0);
  });
}

test("a failed Re-auth request stays on the profile and can be retried", async ({ page }) => {
  await mockProfile(page);
  const githubRequests = await mockGitHub(page);
  let attempts = 0;
  await page.route(LINK_PATH, (route) => {
    attempts++;
    return attempts === 1
      ? route.fulfill({
          status: 503,
          json: { code: "DependencyUnavailable", detail: "GitHub is temporarily unavailable." },
        })
      : route.fulfill({ json: INSTALL_LINK });
  });
  await page.goto(PROFILE_URL);
  await page.getByRole("button", { name: "Re-auth", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("GitHub is temporarily unavailable.");
  await expect(page).toHaveURL(/\/auth\?key=shared-github$/);
  expect(attempts).toBe(1);
  expect(githubRequests).toEqual([]);
  await page.getByRole("button", { name: "Retry", exact: true }).click();
  await expect(page).toHaveURL(INSTALL_URL);
  expect(attempts).toBe(2);
});

test("Re-auth session expiry unmounts the profile without opening GitHub", async ({ page }) => {
  await mockProfile(page);
  const githubRequests = await mockGitHub(page);
  await page.route(LINK_PATH, (route) =>
    route.fulfill({
      status: 401,
      json: { code: "AuthenticationFailed", detail: "Session expired" },
    }),
  );
  await page.goto(PROFILE_URL);
  await page.getByRole("button", { name: "Re-auth", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Sign in to Shaula" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "shared-github", exact: true })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Re-auth", exact: true })).toHaveCount(0);
  expect(githubRequests).toEqual([]);
});

test("Re-auth cannot navigate using a revision replaced by the current profile poll", async ({
  page,
}) => {
  await page.clock.install();
  await mockProfile(page);
  const githubRequests = await mockGitHub(page);
  let current = V2_PROFILE;
  await page.route("**/api/v1/github-auth-profiles/shared-github", (route) =>
    route.fulfill({ headers: { etag: '"auth-inc:3"' }, json: current }),
  );
  let pending: Route | undefined;
  await page.route(LINK_PATH, (route) => {
    pending = route;
  });
  await page.goto(PROFILE_URL);
  await page.getByRole("button", { name: "Re-auth", exact: true }).click();
  await expect.poll(() => pending !== undefined).toBe(true);
  current = {
    ...V2_PROFILE,
    activeRevision: 3,
    desiredRevision: 3,
    active: { ...V2_PROFILE.active, revision: 3 },
  };
  await page.clock.fastForward(10_001);
  await expect(
    page.getByRole("heading", { name: "Account bindings of r3", exact: true }),
  ).toBeVisible();
  await pending!.fulfill({ json: INSTALL_LINK });
  await expect(page.getByRole("alert")).toBeVisible();
  await expect(page).toHaveURL(/\/auth\?key=shared-github$/);
  expect(githubRequests).toEqual([]);
});

const invalidLinks = [
  ["another host", { ...INSTALL_LINK, url: "https://example.com/apps/shaula/installations/new" }],
  ["insecure HTTP", { ...INSTALL_LINK, url: INSTALL_URL.replace("https:", "http:") }],
  ["unexpected query", { ...INSTALL_LINK, url: `${INSTALL_URL}?redirect=https://example.com` }],
  ["encoded slug", { ...INSTALL_LINK, url: "https://github.com/apps/%73haula/installations/new" }],
  ["candidate App ID", { ...INSTALL_LINK, appId: "9999999" }],
  ["candidate revision", { ...INSTALL_LINK, revision: V2_PROFILE.desiredRevision }],
  ["another incarnation", { ...INSTALL_LINK, incarnation: "recreated-auth" }],
] as const;

for (const [description, link] of invalidLinks) {
  test(`Re-auth rejects an installation link with ${description}`, async ({ page }) => {
    await mockProfile(page, {
      ...V2_PROFILE,
      app_id: "9999999",
      desired: { ...V2_PROFILE.desired, app_id: "9999999" },
    });
    const githubRequests = await mockGitHub(page);
    // Intercept even a rejected host if the URL guard regresses.
    await page.route("https://example.com/**", (route) => route.fulfill({ body: "wrong host" }));
    await page.route("http://github.com/**", (route) => route.fulfill({ body: "insecure URL" }));
    await page.route(LINK_PATH, (route) => route.fulfill({ json: link }));
    await page.goto(PROFILE_URL);
    await page.getByRole("button", { name: "Re-auth", exact: true }).click();
    await expect(page.getByRole("alert")).toBeVisible();
    await expect(page.getByRole("button", { name: "Retry", exact: true })).toBeEnabled();
    await expect(page).toHaveURL(/\/auth\?key=shared-github$/);
    expect(githubRequests).toEqual([]);
  });
}

test("Re-auth, rotation and retirement fit the mobile profile header", async ({
  page,
}, testInfo) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await mockProfile(page);
  await page.goto(PROFILE_URL);
  const header = page.locator(".section-heading").filter({
    has: page.getByRole("heading", { name: "shared-github", exact: true }),
  });
  await header.scrollIntoViewIfNeeded();
  for (const name of [
    "Re-auth",
    "Edit target policy",
    "Rotate credential",
    "Retire authentication profile",
  ]) {
    const button = header.getByRole("button", { name, exact: true });
    await expect(button).toBeVisible();
    const bounds = await button.boundingBox();
    expect(bounds).not.toBeNull();
    expect(bounds!.x).toBeGreaterThanOrEqual(0);
    expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(390);
  }
  await expect
    .poll(() => page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth))
    .toBe(true);
  await page.screenshot({
    path: testInfo.outputPath("auth-reauth-mobile.png"),
    fullPage: true,
    animations: "disabled",
  });
});
