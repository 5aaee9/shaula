import type { Page } from "@playwright/test";
import { V2_PROFILE } from "./auth-fixtures";
import { mockApi, scopes } from "./fixtures";

export const AUTH_PATH = "/api/v1/github-auth-profiles/shared-github";
export const POLICY_PAGE = "/auth/shared-github/targets/edit";
export const ROTATE_PAGE = "/auth/shared-github/rotate";
export const AUTH_ETAG = '"auth-inc:2"';
export const ACCEPTED_POLICY = { changeId: "policy-change", state: "Accepted", revision: 3 };

export async function mockAuthPage(page: Page, profile: object = V2_PROFILE, permissions = scopes) {
  await mockApi(page, permissions);
  await page.route(`**${AUTH_PATH}`, (route) =>
    route.fulfill({ headers: { etag: AUTH_ETAG }, json: profile }),
  );
  await page.route(`**${AUTH_PATH}/impact`, (route) =>
    route.fulfill({ json: { desiredRevision: 2, liveFleets: [] } }),
  );
}
