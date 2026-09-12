import { expect, test } from "@playwright/test";
import { authProfile, fleet, mockApi, profile, scopes } from "./fixtures";

for (const failure of ["unavailable lists", "missing permissions"] as const) {
  test(`${failure} preserve both original references when saving capacity`, async ({ page }) => {
    await mockApi(
      page,
      failure === "missing permissions"
        ? scopes.filter((scope) => !["auth.read", "template.read"].includes(scope))
        : scopes,
    );
    if (failure === "unavailable lists") {
      for (const endpoint of ["github-auth-profiles", "template-profiles"]) {
        await page.route(`**/api/v1/${endpoint}`, (route) =>
          route.fulfill({
            status: 503,
            json: { code: "Unavailable", detail: `${endpoint} unavailable` },
          }),
        );
      }
    }
    const current = fleet("linux-build");
    let writes = 0;
    await page.route("**/api/v1/fleets/linux-build", (route) => {
      if (route.request().method() === "GET")
        return route.fulfill({ json: current, headers: { etag: '"original:2"' } });
      writes += 1;
      const body = route.request().postDataJSON();
      expect(body.github.auth_profile_ref).toBe("github-build");
      expect(body.template_profile_ref).toEqual(current.spec.template_profile_ref);
      expect(body.capacity.max_runners).toBe(12);
      expect(route.request().headers()["if-match"]).toBe('"original:2"');
      return route.fulfill({
        status: 202,
        json: { changeId: "capacity-only", state: "Accepted", revision: 3 },
      });
    });
    await page.goto("/fleets/linux-build");
    await page.getByRole("button", { name: "Edit fleet" }).click();
    await expect(page.getByLabel("GitHub authentication profile", { exact: true })).toHaveValue(
      "github-build",
    );
    await expect(page.getByLabel("Template profile", { exact: true })).toHaveValue(
      "kubernetes-linux",
    );
    await expect(page.getByRole("dialog")).toContainText(
      failure === "missing permissions"
        ? "auth.read permission is required"
        : "github-auth-profiles unavailable",
    );
    await page.getByLabel("Maximum runners", { exact: true }).fill("12");
    await page.getByRole("button", { name: "Save changes", exact: true }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    expect(writes).toBe(1);
  });
}

test("removed originals can be restored after switching both references", async ({ page }) => {
  await mockApi(page);
  await page.route("**/api/v1/github-auth-profiles", (route) =>
    route.fulfill({
      json: { profiles: [{ ...authProfile, key: "replacement-auth" }] },
    }),
  );
  await page.route("**/api/v1/template-profiles", (route) =>
    route.fulfill({
      json: { profiles: [profile("replacement-template")] },
    }),
  );
  const current = fleet("linux-build");
  await page.route("**/api/v1/fleets/linux-build", (route) => {
    if (route.request().method() === "GET")
      return route.fulfill({ json: current, headers: { etag: '"original:2"' } });
    const body = route.request().postDataJSON();
    expect(body.github.auth_profile_ref).toBe("github-build");
    expect(body.template_profile_ref).toEqual(current.spec.template_profile_ref);
    expect(body.template_inputs).toEqual({});
    return route.fulfill({
      status: 202,
      json: { changeId: "restored", state: "Accepted", revision: 3 },
    });
  });
  await page.goto("/fleets/linux-build");
  await page.getByRole("button", { name: "Edit fleet" }).click();
  const auth = page.getByLabel("GitHub authentication profile", { exact: true });
  const template = page.getByLabel("Template profile", { exact: true });
  await expect(template).toBeEnabled();
  await auth.selectOption("replacement-auth");
  await template.selectOption("replacement-template");
  await page
    .getByRole("button", { name: "Restore original template and inputs", exact: true })
    .click();
  await page.getByRole("button", { name: "Restore original authentication", exact: true }).click();
  await expect(auth).toHaveValue("github-build");
  await expect(template).toHaveValue("kubernetes-linux");
  await page.getByRole("button", { name: "Save changes", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
});

test("same-key revision changes remain saveable when the profile list refresh fails", async ({
  page,
}) => {
  await mockApi(page);
  await page.route("**/api/v1/template-profiles/kubernetes-linux", (route) =>
    route.fulfill({
      json: profile("kubernetes-linux", 4),
    }),
  );
  await page.goto("/fleets/linux-build");
  await page.getByRole("button", { name: "Edit fleet" }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("kubernetes-linux · Revision 3");
  await dialog.getByRole("button", { name: "Load latest Active", exact: true }).click();
  await expect(dialog).toContainText("kubernetes-linux · Revision 4");
  await page.route("**/api/v1/template-profiles", (route) =>
    route.fulfill({
      status: 503,
      json: { code: "Unavailable", detail: "Latest list unavailable" },
    }),
  );
  await dialog.getByRole("button", { name: "Refresh Template profiles", exact: true }).click();
  await expect(dialog).toContainText("Latest list unavailable");
  // The Fleet reference is a bare key and follows the latest Active revision;
  // a failed profile-list refresh does not invalidate the already loaded
  // contract or block unrelated Fleet edits.
  await expect(dialog.getByRole("button", { name: "Save changes", exact: true })).toBeEnabled();
});
