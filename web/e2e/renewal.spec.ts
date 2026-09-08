import { expect, test } from "@playwright/test";

test("Provider renewal preserves the same page and draft through rotation and ID Token omission", async ({
  page,
  request,
  context,
}) => {
  await request.post("/__test/provider", {
    data: { issue_refresh: true, expires_in: 2 },
  });
  await page.goto("/auth?key=browser-renewal");
  await expect(page.getByRole("heading", { name: "Authentication", exact: true })).toBeVisible();
  await page.evaluate(() => history.replaceState(null, "", `${location.href}#editor`));
  await page.getByRole("button", { name: "Create profile", exact: true }).click();
  await page.getByLabel("Profile key", { exact: true }).fill("browser-renewal");
  await page.getByLabel("App ID", { exact: true }).fill("12345");
  const draft = page.getByLabel("Private key (PEM)", { exact: true });
  await draft.fill("browser-draft-private-key");
  await page.getByRole("textbox", { name: "Owner", exact: true }).fill("5aaee9");
  const originalInput = await draft.elementHandle();
  const originalUrl = page.url();
  const cookie = (await context.cookies()).find((value) => value.name === "__Host-shaula-session");
  expect(cookie).toMatchObject({ httpOnly: true, secure: true, sameSite: "Lax", expires: -1 });
  const csrf = await page.evaluate(async () => {
    const response = await fetch("/api/v1/session");
    return response.headers.get("x-csrf-token");
  });
  expect(csrf).toBeTruthy();
  const before = await (await request.get("/__test/provider")).json();
  let navigations = 0;
  page.on("framenavigated", (frame) => {
    if (frame === page.mainFrame()) navigations += 1;
  });

  // Real server clock: expire the two-second lease, then issue ordinary browser requests.
  await page.waitForTimeout(2100);
  const refreshed = page.waitForResponse("**/api/v1/github-auth-profiles");
  // The credential Dialog stays mounted while these requests renew the session.
  const sessionResult = await page.evaluate(async () => {
    const responses = await Promise.all([
      fetch("/api/v1/session"),
      fetch("/api/v1/github-auth-profiles"),
      fetch("/readyz"),
    ]);
    return responses.map((response) => ({
      status: response.status,
      csrf: response.headers.get("x-csrf-token"),
    }));
  });
  expect((await refreshed).status()).toBe(200);
  expect(sessionResult[0]).toEqual({ status: 200, csrf });
  expect(sessionResult.map((response) => response.status)).toEqual([200, 200, 200]);
  expect((await (await request.get("/__test/provider")).json()).refreshRequests).toBe(
    before.refreshRequests + 1,
  );

  // The rotated grant must work even when the next conforming response omits its ID Token.
  await request.post("/__test/provider", {
    data: { issue_refresh: true, expires_in: 3600, omit_refresh_id_token: true },
  });
  await page.waitForTimeout(2100);
  const mutationPath = "/api/v1/github-auth-profiles/browser-renewal";
  const mutationStatus = await page.evaluate(
    async ({ path, csrf }) => {
      const response = await fetch(path, {
        method: "PUT",
        headers: {
          "content-type": "application/json",
          "x-csrf-token": csrf!,
          "if-none-match": "*",
          "idempotency-key": crypto.randomUUID(),
        },
        body: "{}",
      });
      return response.status;
    },
    { path: mutationPath, csrf },
  );
  // The original request reaches real DTO validation once; no resource is created.
  expect(mutationStatus).toBe(422);
  const after = await (await request.get("/__test/provider")).json();
  expect(after.refreshRequests).toBe(before.refreshRequests + 2);
  const mutationAttempts = (requests: string[][]) =>
    requests.filter(([method, path]) => method === "PUT" && path === mutationPath).length;
  expect(mutationAttempts(after.requests) - mutationAttempts(before.requests)).toBe(1);
  expect(navigations).toBe(0);
  expect(page.url()).toBe(originalUrl);
  expect(await originalInput!.evaluate((node) => node.isConnected)).toBe(true);
  await expect(draft).toHaveValue("browser-draft-private-key");
  await expect(page.getByRole("textbox", { name: "Owner", exact: true })).toHaveValue("5aaee9");
  expect((await context.cookies()).find((value) => value.name === cookie!.name)).toEqual(cookie);
  expect(await page.evaluate(() => [localStorage.length, sessionStorage.length])).toEqual([0, 0]);
  expect(
    await page.evaluate(async () => (await fetch("/api/v1/session")).headers.get("x-csrf-token")),
  ).toBe(csrf);
});
