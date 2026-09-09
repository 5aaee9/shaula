import { expect, test } from "@playwright/test";
import { mockApi, scopes } from "./fixtures";
import { UNSUPPORTED_PROFILE, V2_PROFILE } from "./auth-fixtures";

test("authentication lists existing connections without a key and opens their details", async ({
  page,
}) => {
  await mockApi(page);
  await page.route("**/api/v1/github-auth-profiles", (route) =>
    route.fulfill({ json: { profiles: [V2_PROFILE, UNSUPPORTED_PROFILE] } }),
  );
  await page.route("**/api/v1/github-auth-profiles/shared-github", (route) =>
    route.fulfill({ headers: { etag: '"auth-inc:2"' }, json: V2_PROFILE }),
  );
  await page.goto("/auth");
  const connection = page.getByRole("row").filter({ hasText: "shared-github" });
  await expect(connection).toBeVisible();
  await expect(connection.getByText("GitHub App", { exact: true })).toBeVisible();
  await expect(connection.getByText("Active", { exact: true })).toBeVisible();
  await expect(connection.getByText("5aaee9 (user repositories)")).toBeVisible();
  await expect(page.getByRole("row").filter({ hasText: "old-auth" })).toBeVisible();
  await connection.getByRole("button", { name: "shared-github", exact: true }).click();
  await expect(page).toHaveURL(/\/auth\?key=shared-github$/);
  await expect(connection).toHaveAttribute("data-state", "selected");
  await expect(page.getByRole("heading", { name: "shared-github", exact: true })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Account bindings of r1" })).toBeVisible();
});

test("authentication search filters connections and refresh discovers newly added profiles", async ({
  page,
}) => {
  await mockApi(page);
  let profiles = [V2_PROFILE];
  await page.route("**/api/v1/github-auth-profiles", (route) =>
    route.fulfill({ json: { profiles } }),
  );
  await page.goto("/auth");
  await expect(page.getByRole("button", { name: "shared-github", exact: true })).toBeVisible();
  const search = page.getByRole("textbox", { name: "Search authentication connections" });
  await search.fill("SHARED");
  await expect(page.getByRole("button", { name: "shared-github", exact: true })).toBeVisible();
  await search.fill("missing");
  await expect(page.getByText("No matching connections", { exact: true })).toBeVisible();
  profiles = [{ ...V2_PROFILE, key: "new-connection" }];
  await search.fill("");
  await page.getByRole("button", { name: "Refresh authentication connections" }).click();
  await expect(page.getByRole("button", { name: "new-connection", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "shared-github", exact: true })).toHaveCount(0);
});

test("empty authentication inventory is distinct from a failed list request", async ({ page }) => {
  await mockApi(page);
  await page.route("**/api/v1/github-auth-profiles", (route) =>
    route.fulfill({ json: { profiles: [] } }),
  );
  await page.goto("/auth");
  await expect(page.getByText("No authentication connections yet", { exact: true })).toBeVisible();
  await page.route("**/api/v1/github-auth-profiles", (route) =>
    route.fulfill({
      status: 403,
      json: { code: "PermissionDenied", detail: "Missing auth.read" },
    }),
  );
  await page.getByRole("button", { name: "Refresh authentication connections" }).click();
  await expect(page.getByText("You do not have permission for this operation.")).toBeVisible();
  await expect(page.getByText("No authentication connections yet", { exact: true })).toHaveCount(0);
});

test("authentication inventory is not requested without auth.read", async ({ page }) => {
  await mockApi(
    page,
    scopes.filter((scope) => scope !== "auth.read"),
  );
  const requests: string[] = [];
  page.on("request", (request) => {
    if (request.url().includes("/github-auth-profiles")) requests.push(request.url());
  });
  await page.goto("/auth?key=shared-github");
  await expect(page.getByText("Authentication profile read permission is required.")).toBeVisible();
  expect(requests).toEqual([]);
});

test("a failed authentication inventory does not block a bookmarked connection", async ({
  page,
}) => {
  await mockApi(page);
  await page.route("**/api/v1/github-auth-profiles", (route) =>
    route.fulfill({ status: 503, json: { code: "Internal", detail: "Inventory unavailable" } }),
  );
  await page.route("**/api/v1/github-auth-profiles/shared-github", (route) =>
    route.fulfill({ headers: { etag: '"auth-inc:2"' }, json: V2_PROFILE }),
  );
  await page.goto("/auth?key=shared-github");
  await expect(page.getByText("Inventory unavailable", { exact: true })).toBeVisible();
  await expect(page.getByRole("heading", { name: "shared-github", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Rotate credential", exact: true })).toBeEnabled();
});

test("connection summaries show active targets without promoting candidate policy", async ({
  page,
}) => {
  await mockApi(page);
  const candidate = {
    ...V2_PROFILE,
    desired: {
      ...V2_PROFILE.desired,
      target_policy: [{ kind: "organization", owner: "candidate-only" }],
    },
  };
  await page.route("**/api/v1/github-auth-profiles", (route) =>
    route.fulfill({
      json: {
        profiles: [
          candidate,
          {
            ...candidate,
            key: "pending-first-revision",
            status: "Validating",
            activeRevision: null,
            active: undefined,
          },
        ],
      },
    }),
  );
  await page.goto("/auth");
  const active = page.getByRole("row").filter({ hasText: "shared-github" });
  await expect(active.getByText("5aaee9 (user repositories)")).toBeVisible();
  await expect(page.getByText("candidate-only (org runners)")).toHaveCount(0);
  const pending = page.getByRole("row").filter({ hasText: "pending-first-revision" });
  await expect(pending).toBeVisible();
  await expect(pending.getByText("5aaee9 (user repositories)")).toHaveCount(0);
});
