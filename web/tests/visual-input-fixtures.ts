import { expect, type Page } from "@playwright/test";

export const artifactDigest = `sha256:${"a".repeat(64)}`;

export function field(
  key: string,
  values: string[],
  required = false,
  label = key,
  description = "",
) {
  return {
    key,
    label,
    description,
    required,
    options: values.map((valueJson) => ({ valueJson })),
  };
}

export function contract(
  fields: ReturnType<typeof field>[] = [],
  profileKey = "kubernetes-linux",
  revision = 3,
) {
  return {
    version: 1,
    profileKey,
    incarnation: "template-inc",
    revision,
    artifactDigest,
    mode: "fields",
    fields,
  };
}

export async function mockContract(page: Page, inputContract: ReturnType<typeof contract>) {
  const { profileKey, revision } = inputContract;
  await page.route(
    `**/api/v1/template-profiles/${profileKey}/revisions/${revision}/input-contract`,
    (route) => route.fulfill({ json: inputContract }),
  );
}

export async function openCreate(page: Page) {
  await page.goto("/fleets");
  await page.getByRole("button", { name: "Create fleet" }).click();
  await page.getByLabel("Fleet key", { exact: true }).fill("visual-build");
  await page.getByLabel("Owner", { exact: true }).fill("acme");
  await page.getByLabel("GitHub authentication profile", { exact: true }).fill("github-build");
}

export async function loadTemplate(page: Page, key = "kubernetes-linux") {
  await page.getByLabel("Template profile", { exact: true }).fill(key);
  await page.getByLabel("Template profile", { exact: true }).press("Tab");
}

export async function acceptCreate(page: Page, check: (body: string) => void) {
  let writes = 0;
  await page.route("**/api/v1/fleets/visual-build", (route) => {
    writes += 1;
    check(route.request().postData()!);
    expect(route.request().headers()["if-none-match"]).toBe("*");
    return route.fulfill({
      status: 202,
      json: { changeId: "visual-create", state: "Accepted", revision: 1 },
    });
  });
  await page.getByRole("dialog").getByRole("button", { name: "Create fleet" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(writes).toBe(1);
}
