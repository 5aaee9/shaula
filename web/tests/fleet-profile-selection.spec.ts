import { expect, test } from "@playwright/test";
import { authProfile, mockApi, profile, scopes } from "./fixtures";
import { contract, field, loadTemplate, mockContract, openCreate } from "./visual-input-fixtures";

test("server choices are sorted, never preselected, and retain an older Active candidate", async ({
  page,
}) => {
  await mockApi(page);
  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  await page.route("**/api/v1/github-auth-profiles", async (route) => {
    await gate;
    return route.fulfill({
      json: {
        profiles: [
          { ...authProfile, key: "z-retiring", status: "Retiring" },
          {
            ...authProfile,
            key: "a-validating",
            status: "Validating",
            desiredRevision: 2,
            health: "Unknown",
          },
          { ...authProfile, key: "m-inactive", activeRevision: null },
        ],
      },
    });
  });
  await page.goto("/fleets");
  await page.getByRole("button", { name: "Create fleet" }).click();
  const auth = page.getByLabel("GitHub authentication profile", { exact: true });
  const template = page.getByLabel("Template profile", { exact: true });
  await expect(auth).toBeDisabled();
  await expect(page.getByRole("dialog")).toContainText("Loading github authentication profiles");
  release();
  await expect(auth).toBeEnabled();
  await expect(auth).toHaveValue("");
  await expect(template).toHaveValue("");
  expect(
    await auth
      .locator("option")
      .evaluateAll((options) => options.map((option) => (option as HTMLOptionElement).value)),
  ).toEqual(["", "a-validating", "m-inactive", "z-retiring"]);
  await expect(auth.locator('option[value="a-validating"]')).toBeEnabled();
  await expect(auth.locator('option[value="a-validating"]')).toContainText("r1 · Validating");
  await expect(auth.locator('option[value="m-inactive"]')).toBeDisabled();
  await expect(auth.locator('option[value="z-retiring"]')).toBeDisabled();
  await auth.focus();
  await auth.press("ArrowDown");
  await auth.press("Enter");
  await expect(auth).toHaveValue("a-validating");
});

test("empty, unavailable, forbidden and failed lists stay distinct and refresh recovers", async ({
  page,
}) => {
  await mockApi(page);
  let recovered = false;
  await page.route("**/api/v1/github-auth-profiles", (route) =>
    route.fulfill(
      recovered
        ? { json: { profiles: [authProfile] } }
        : { status: 403, json: { code: "Forbidden", detail: "Authentication list forbidden" } },
    ),
  );
  await page.route("**/api/v1/template-profiles", (route) =>
    route.fulfill({
      json: { profiles: recovered ? [profile("unvalidated", null, "Validating")] : [] },
    }),
  );
  await page.goto("/fleets");
  await page.getByRole("button", { name: "Create fleet" }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("You do not have permission for this operation.");
  await expect(dialog).toContainText("No template profiles yet");
  await expect(dialog.getByRole("button", { name: "Create fleet", exact: true })).toBeDisabled();
  recovered = true;
  await dialog
    .getByRole("button", { name: "Refresh GitHub authentication profiles", exact: true })
    .click();
  await dialog.getByRole("button", { name: "Refresh Template profiles", exact: true }).click();
  await expect(page.getByLabel("GitHub authentication profile", { exact: true })).toBeEnabled();
  await expect(dialog).toContainText("No profiles are available for new references");
  await expect(
    page.getByLabel("Template profile", { exact: true }).locator('option[value="unvalidated"]'),
  ).toBeDisabled();
  await page.route("**/api/v1/template-profiles", (route) =>
    route.fulfill({
      status: 503,
      json: { code: "Unavailable", detail: "Template registry unavailable" },
    }),
  );
  await dialog.getByRole("button", { name: "Refresh Template profiles", exact: true }).click();
  await expect(dialog).toContainText("Template registry unavailable");
  await expect(dialog).not.toContainText("No template profiles yet");
});

for (const list of ["github-auth-profiles", "template-profiles"] as const) {
  test(`${list} refresh failure blocks a new reference without erasing its draft`, async ({
    page,
  }) => {
    await mockApi(page);
    await mockContract(page, contract([field("image", ['"approved"'], true, "Runner image")]));
    await openCreate(page);
    await loadTemplate(page);
    await page.getByRole("combobox", { name: "Runner image", exact: true }).selectOption("0");
    const dialog = page.getByRole("dialog");
    const submit = dialog.getByRole("button", { name: "Create fleet", exact: true });
    await expect(submit).toBeEnabled();
    let fail = true;
    await page.route(`**/api/v1/${list}`, (route) =>
      route.fulfill(
        fail
          ? { status: 503, json: { code: "Unavailable", detail: "Registry read failed" } }
          : {
              json: {
                profiles:
                  list === "template-profiles" ? [profile("kubernetes-linux")] : [authProfile],
              },
            },
      ),
    );
    const refresh = dialog.getByRole("button", {
      name:
        list === "template-profiles"
          ? "Refresh Template profiles"
          : "Refresh GitHub authentication profiles",
      exact: true,
    });
    await refresh.click();
    await expect(dialog).toContainText("Registry read failed");
    await expect(submit).toBeDisabled();
    await expect(page.getByLabel("GitHub authentication profile", { exact: true })).toHaveValue(
      "github-build",
    );
    await expect(page.getByLabel("Template profile", { exact: true })).toHaveValue(
      "kubernetes-linux",
    );
    await expect(page.getByRole("combobox", { name: "Runner image", exact: true })).toHaveValue(
      "0",
    );
    fail = false;
    await refresh.click();
    await expect(submit).toBeEnabled();
    await expect(page.getByRole("combobox", { name: "Runner image", exact: true })).toHaveValue(
      "0",
    );
  });
}

test("retiring or removed selections remain visible but cannot create a new fleet", async ({
  page,
}) => {
  await mockApi(page);
  await openCreate(page);
  await loadTemplate(page);
  const dialog = page.getByRole("dialog");
  const submit = dialog.getByRole("button", { name: "Create fleet", exact: true });
  await expect(submit).toBeEnabled();
  await page.route("**/api/v1/github-auth-profiles", (route) =>
    route.fulfill({ json: { profiles: [] } }),
  );
  await page.route("**/api/v1/template-profiles", (route) =>
    route.fulfill({
      json: { profiles: [profile("kubernetes-linux", 3, "Retiring")] },
    }),
  );
  await dialog
    .getByRole("button", { name: "Refresh GitHub authentication profiles", exact: true })
    .click();
  await dialog.getByRole("button", { name: "Refresh Template profiles", exact: true }).click();
  await expect(dialog).toContainText("github-build: This profile is no longer in the list");
  await expect(dialog).toContainText("kubernetes-linux: Retirement prevents new references");
  await expect(submit).toBeDisabled();
  await expect(page.getByLabel("GitHub authentication profile", { exact: true })).toHaveValue(
    "github-build",
  );
  await expect(page.getByLabel("Template profile", { exact: true })).toHaveValue(
    "kubernetes-linux",
  );
});

test("missing read permissions issue no profile list requests and prevent creation", async ({
  page,
}) => {
  await mockApi(
    page,
    scopes.filter((scope) => !["auth.read", "template.read"].includes(scope)),
  );
  const reads: string[] = [];
  page.on("request", (request) => {
    if (/\/api\/v1\/(github-auth-profiles|template-profiles)$/.test(request.url()))
      reads.push(request.url());
  });
  await page.goto("/fleets");
  await page.getByRole("button", { name: "Create fleet" }).click();
  await expect(page.getByLabel("GitHub authentication profile", { exact: true })).toBeDisabled();
  await expect(page.getByLabel("Template profile", { exact: true })).toBeDisabled();
  await expect(page.getByRole("dialog")).toContainText("auth.read permission is required");
  await expect(page.getByRole("dialog")).toContainText("template.read permission is required");
  await expect(
    page.getByRole("dialog").getByRole("button", { name: "Create fleet", exact: true }),
  ).toBeDisabled();
  expect(reads).toEqual([]);
});

test("a template retired after the list read cannot load a new input contract", async ({
  page,
}) => {
  await mockApi(page);
  let contractReads = 0;
  await page.route("**/api/v1/template-profiles/kubernetes-linux", (route) =>
    route.fulfill({
      json: profile("kubernetes-linux", 3, "Retiring"),
    }),
  );
  await page.route("**/api/v1/template-profiles/**/input-contract", (route) => {
    contractReads += 1;
    return route.fulfill({ json: contract() });
  });
  await openCreate(page);
  await loadTemplate(page);
  await expect(page.getByRole("dialog")).toContainText(/retir/i);
  await expect(
    page.getByRole("dialog").getByRole("button", { name: "Create fleet", exact: true }),
  ).toBeDisabled();
  expect(contractReads).toBe(0);
});

test("long server keys fit a narrow dialog and selections remain accessible", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await mockApi(page);
  const key = `profile-${"a".repeat(55)}`;
  await page.route("**/api/v1/github-auth-profiles", (route) =>
    route.fulfill({
      json: { profiles: [{ ...authProfile, key }] },
    }),
  );
  await page.goto("/fleets");
  await page.getByRole("button", { name: "Create fleet" }).click();
  const auth = page.getByRole("combobox", { name: "GitHub authentication profile", exact: true });
  await auth.selectOption(key);
  await expect(auth).toHaveValue(key);
  const dialog = page.getByRole("dialog");
  const bounds = await dialog.boundingBox();
  expect(bounds!.x).toBeGreaterThanOrEqual(0);
  expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(390);
  expect(await dialog.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true);
  await page.screenshot({ path: "test-results/fleet-profile-mobile.png", animations: "disabled" });
});
