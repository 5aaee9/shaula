import { expect, test } from "@playwright/test";
import { V2_PROFILE } from "./auth-fixtures";
import {
  ACCEPTED_POLICY,
  AUTH_ETAG,
  AUTH_PATH,
  POLICY_PAGE,
  ROTATE_PAGE,
  mockAuthPage,
} from "./auth-page-fixtures";

for (const expanded of [false, true]) {
  test(`policy publication ${expanded ? "adds a target" : "revalidates unchanged targets"} without credentials`, async ({
    page,
  }) => {
    await mockAuthPage(page, {
      ...V2_PROFILE,
      desired: {
        ...V2_PROFILE.desired,
        app_id: "9999999",
        target_policy: [{ kind: "organization", owner: "candidate-only" }],
      },
    });
    let submitted: unknown;
    await page.route(`**${AUTH_PATH}/policy-updates`, (route) => {
      const request = route.request();
      expect(request.method()).toBe("POST");
      expect(request.headers()["if-match"]).toBe(AUTH_ETAG);
      expect(request.headers()["x-csrf-token"]).toBe("test-session-csrf");
      expect(request.headers()["idempotency-key"]).toBeTruthy();
      submitted = request.postDataJSON();
      return route.fulfill({ status: 202, json: ACCEPTED_POLICY });
    });
    await page.goto(POLICY_PAGE);
    await expect(
      page.getByRole("heading", { name: "Edit target policy for shared-github" }),
    ).toBeVisible();
    await expect(page.getByLabel("Private key (PEM)", { exact: true })).toHaveCount(0);
    await expect(page.getByLabel("App ID", { exact: true })).toHaveCount(0);
    await expect(page.getByLabel("Owner", { exact: true }).first()).toHaveValue("Indexyz");
    await expect(page.getByLabel("Owner", { exact: true }).last()).toHaveValue("5aaee9");
    await expect(page.getByRole("region", { name: "Policy change preview" })).not.toContainText(
      "credential rotation",
    );
    if (expanded) {
      await page.getByRole("button", { name: "Add selector" }).click();
      await page.getByLabel("Owner", { exact: true }).last().fill("another-org");
    }
    await page.getByRole("button", { name: "Publish policy", exact: true }).click();
    await expect(page).toHaveURL(/\/auth\?key=shared-github$/);
    await expect(page.getByText("Change for shared-github", { exact: true })).toBeVisible();
    expect(submitted).toEqual({
      base_revision: 1,
      target_policy: [
        ...V2_PROFILE.active.target_policy,
        ...(expanded ? [{ kind: "organization", owner: "another-org" }] : []),
      ],
    });
    expect(await page.evaluate(() => [localStorage.length, sessionStorage.length])).toEqual([0, 0]);
  });
}

for (const status of [409, 412, 422]) {
  test(`policy publication ${status} keeps the reviewed base and input for explicit retry`, async ({
    page,
  }) => {
    await mockAuthPage(page);
    const attempts: { body: unknown; etag: string; idempotency: string }[] = [];
    await page.route(`**${AUTH_PATH}/policy-updates`, (route) => {
      const request = route.request();
      attempts.push({
        body: request.postDataJSON(),
        etag: request.headers()["if-match"],
        idempotency: request.headers()["idempotency-key"],
      });
      return route.fulfill({
        status,
        json: { code: "Conflict", detail: "Review the active revision." },
      });
    });
    await page.goto(POLICY_PAGE);
    await page.getByLabel("Owner", { exact: true }).first().fill("reviewed-org");
    const publish = page.getByRole("button", { name: "Publish policy", exact: true });
    await publish.click();
    await expect(page.getByRole("alert")).toBeVisible();
    await expect(page).toHaveURL(new RegExp(`${POLICY_PAGE}$`));
    await expect(page.getByLabel("Owner", { exact: true }).first()).toHaveValue("reviewed-org");
    expect(attempts).toHaveLength(1);
    await publish.click();
    await expect.poll(() => attempts.length).toBe(2);
    expect(attempts[0]).toEqual(attempts[1]);
    expect(attempts[1]).toMatchObject({ etag: AUTH_ETAG, body: { base_revision: 1 } });
  });
}

test("uncertain policy publication retries the exact request and idempotency key", async ({
  page,
}) => {
  await mockAuthPage(page);
  const requests: { body: string | null; key: string }[] = [];
  await page.route(`**${AUTH_PATH}/policy-updates`, (route) => {
    const request = route.request();
    requests.push({ body: request.postData(), key: request.headers()["idempotency-key"] });
    return requests.length === 1
      ? route.abort("failed")
      : route.fulfill({ status: 202, json: ACCEPTED_POLICY });
  });
  await page.goto(POLICY_PAGE);
  const publish = page.getByRole("button", { name: "Publish policy", exact: true });
  await publish.click();
  await expect(page.getByRole("alert")).toBeVisible();
  await publish.click();
  await expect(page).toHaveURL(/\/auth\?key=shared-github$/);
  expect(requests).toHaveLength(2);
  expect(requests[1]).toEqual(requests[0]);
});

test("rotation requires a new secret and retains the active policy without editable selectors", async ({
  page,
}) => {
  await mockAuthPage(page, {
    ...V2_PROFILE,
    desired: {
      ...V2_PROFILE.desired,
      target_policy: [{ kind: "organization", owner: "candidate-only" }],
    },
  });
  let submitted: unknown;
  await page.route(`**${AUTH_PATH}`, (route) => {
    if (route.request().method() === "GET") return route.fallback();
    expect(route.request().method()).toBe("PUT");
    expect(route.request().headers()["if-match"]).toBe(AUTH_ETAG);
    submitted = route.request().postDataJSON();
    return route.fulfill({ status: 202, json: ACCEPTED_POLICY });
  });
  await page.goto(ROTATE_PAGE);
  await expect(
    page.getByRole("heading", { name: "Rotate credential for shared-github" }),
  ).toBeVisible();
  await expect(page.getByLabel("Owner", { exact: true })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Add selector" })).toHaveCount(0);
  const secret = page.getByLabel("Private key (PEM)", { exact: true });
  await expect(secret).toHaveAttribute("autocomplete", "off");
  await page.getByRole("button", { name: "Rotate credential", exact: true }).click();
  await expect(secret).toBeFocused();
  expect(submitted).toBeUndefined();
  await secret.fill("test-rotation-private-key");
  await page.getByRole("button", { name: "Rotate credential", exact: true }).click();
  await expect(page).toHaveURL(/\/auth\?key=shared-github$/);
  expect(submitted).toEqual({
    kind: "github_app",
    schema_version: 2,
    app_id: V2_PROFILE.active.app_id,
    private_key: "test-rotation-private-key",
    target_policy: V2_PROFILE.active.target_policy,
  });
  await expect(page.locator("body")).not.toContainText("test-rotation-private-key");
  expect(await page.evaluate(() => [localStorage.length, sessionStorage.length])).toEqual([0, 0]);
});
