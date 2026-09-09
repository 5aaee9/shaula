import { expect, test, type Route } from "@playwright/test";
import { V2_PROFILE } from "./auth-fixtures";
import { scopes } from "./fixtures";
import {
  ACCEPTED_POLICY,
  AUTH_PATH,
  POLICY_PAGE,
  ROTATE_PAGE,
  mockAuthPage,
} from "./auth-page-fixtures";

const FORM_NAME = "Authentication configuration";

for (const [path, heading] of [
  ["/auth/new", "Create authentication profile"],
  [POLICY_PAGE, "Edit target policy for shared-github"],
  [ROTATE_PAGE, "Rotate credential for shared-github"],
]) {
  test(`authentication page ${path} supports direct navigation and refresh`, async ({ page }) => {
    await mockAuthPage(page);
    await page.goto(path);
    await expect(page.getByRole("heading", { name: heading, exact: true })).toBeVisible();
    await expect(page.getByRole("form", { name: FORM_NAME })).toBeVisible();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await page.reload();
    await expect(page.getByRole("heading", { name: heading, exact: true })).toBeVisible();
    await expect(page.getByRole("form", { name: FORM_NAME })).toBeVisible();
    await page.getByRole("button", { name: "Cancel", exact: true }).click();
    await expect(page).toHaveURL(path === "/auth/new" ? /\/auth$/ : /\/auth\?key=shared-github$/);
  });
}

for (const permission of ["auth.read", "auth.write"]) {
  test(`authentication edit deep links cannot read or publish without ${permission}`, async ({
    page,
  }) => {
    await mockAuthPage(
      page,
      V2_PROFILE,
      scopes.filter((scope) => scope !== permission),
    );
    const requests: string[] = [];
    page.on("request", (request) => {
      if (request.url().includes("github-auth-profiles")) requests.push(request.url());
    });
    for (const path of [POLICY_PAGE, ROTATE_PAGE]) {
      await page.goto(path);
      await expect(page.getByRole("alert")).toContainText("permission");
      await expect(page.getByRole("form", { name: FORM_NAME })).toHaveCount(0);
    }
    expect(requests).toEqual([]);
  });
}

test("reopening a cancelled policy edit loads a fresh active revision and clears the old draft", async ({
  page,
}) => {
  await mockAuthPage(page);
  let current = V2_PROFILE;
  let etag = '"auth-inc:2"';
  await page.route(`**${AUTH_PATH}`, (route) =>
    route.fulfill({ headers: { etag }, json: current }),
  );
  let submitted: unknown;
  let submittedEtag: string | undefined;
  await page.route(`**${AUTH_PATH}/policy-updates`, (route) => {
    submitted = route.request().postDataJSON();
    submittedEtag = route.request().headers()["if-match"];
    return route.fulfill({ status: 202, json: ACCEPTED_POLICY });
  });
  await page.goto("/auth?key=shared-github");
  await page.getByRole("button", { name: "Edit target policy", exact: true }).click();
  await page.getByLabel("Owner", { exact: true }).first().fill("discarded-draft");
  current = {
    ...V2_PROFILE,
    desiredRevision: 4,
    activeRevision: 3,
    active: {
      ...V2_PROFILE.active,
      revision: 3,
      target_policy: [{ kind: "organization", owner: "new-active-org" }],
    },
  };
  etag = '"auth-inc:4"';
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  await page.getByRole("button", { name: "Edit target policy", exact: true }).click();
  await expect(page.getByLabel("Owner", { exact: true })).toHaveValue("new-active-org");
  await page.getByRole("button", { name: "Publish policy", exact: true }).click();
  await expect(page).toHaveURL(/\/auth\?key=shared-github$/);
  expect(submittedEtag).toBe(etag);
  expect(submitted).toEqual({ base_revision: 3, target_policy: current.active.target_policy });
});

test("leaving credential rotation discards its secret before the next visit", async ({ page }) => {
  await mockAuthPage(page);
  await page.goto("/auth?key=shared-github");
  await page.getByRole("button", { name: "Rotate credential", exact: true }).click();
  const secret = page.getByLabel("Private key (PEM)", { exact: true });
  await secret.fill("discard-on-navigation");
  await page.goBack();
  await expect(page).toHaveURL(/\/auth\?key=shared-github$/);
  await page.getByRole("button", { name: "Rotate credential", exact: true }).click();
  await expect(secret).toHaveValue("");
  expect(await page.evaluate(() => [localStorage.length, sessionStorage.length])).toEqual([0, 0]);
});

test("a late policy response cannot replace a later credential form", async ({ page }) => {
  await mockAuthPage(page);
  await page.route("**/api/v1/github-auth-profiles", (route) =>
    route.fulfill({ json: { profiles: [V2_PROFILE] } }),
  );
  let pending: Route | undefined;
  await page.route(`**${AUTH_PATH}/policy-updates`, (route) => {
    pending = route;
  });
  await page.goto("/auth?key=shared-github");
  await page.getByRole("button", { name: "Edit target policy", exact: true }).click();
  await page.getByRole("button", { name: "Publish policy", exact: true }).click();
  await expect.poll(() => pending !== undefined).toBe(true);
  await page.getByRole("link", { name: "GitHub authentication", exact: true }).click();
  await page.getByRole("button", { name: "shared-github", exact: true }).click();
  await page.getByRole("button", { name: "Rotate credential", exact: true }).click();
  const secret = page.getByLabel("Private key (PEM)", { exact: true });
  await secret.fill("later-form-draft");
  await pending!.fulfill({ status: 202, json: ACCEPTED_POLICY });
  await expect(page).toHaveURL(new RegExp(`${ROTATE_PAGE}$`));
  await expect(secret).toHaveValue("later-form-draft");
});

test("policy editor fits a narrow viewport with visible publication controls", async ({
  page,
}, testInfo) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await mockAuthPage(page);
  await page.goto(POLICY_PAGE);
  const publish = page.getByRole("button", { name: "Publish policy", exact: true });
  await expect(publish).toBeEnabled();
  const bounds = await publish.boundingBox();
  expect(bounds).not.toBeNull();
  expect(bounds!.x).toBeGreaterThanOrEqual(0);
  expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(390);
  expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(844);
  await expect
    .poll(() => page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth))
    .toBe(true);
  await page.screenshot({
    path: testInfo.outputPath("auth-policy-mobile.png"),
    fullPage: true,
    animations: "disabled",
  });
});
