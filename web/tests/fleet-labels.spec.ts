import { expect, test } from "@playwright/test";
import { fleet, mockApi } from "./fixtures";

for (const { name, draft, expected } of [
  { name: "add", draft: " linux, x64, arm64 ", expected: ["linux", "x64", "arm64"] },
  { name: "remove", draft: "linux", expected: ["linux"] },
  { name: "clear", draft: "", expected: [] },
]) {
  test(`existing fleet labels can ${name} without replacing identity or template inputs`, async ({
    page,
  }) => {
    await mockApi(page);
    const current = fleet("linux-build");
    let writes = 0;
    await page.route("**/api/v1/fleets/linux-build", (route) => {
      if (route.request().method() === "GET")
        return route.fulfill({ headers: { etag: '"original:2"' }, json: current });
      writes += 1;
      expect(route.request().method()).toBe("PUT");
      expect(route.request().headers()["if-match"]).toBe('"original:2"');
      expect(route.request().headers()["idempotency-key"]).toBeTruthy();
      expect(route.request().postDataJSON()).toEqual({
        ...current.spec,
        github: { ...current.spec.github, labels: expected },
      });
      return route.fulfill({
        status: 202,
        json: { changeId: "labels-update", state: "Accepted", revision: 3 },
      });
    });
    await page.goto("/fleets/linux-build");
    await page.getByRole("button", { name: "Edit fleet" }).click();
    await page.getByRole("button", { name: "Advanced settings" }).click();
    await expect(page.getByLabel("Labels", { exact: true })).toBeEnabled();
    await expect(page.getByLabel("Scale set name", { exact: true })).toBeDisabled();
    await expect(page.getByLabel("Runner group", { exact: true })).toBeDisabled();
    await expect(page.getByRole("dialog")).toContainText("Changes sync to GitHub after saving");
    await page.getByLabel("Labels", { exact: true }).fill(draft);
    await page.getByRole("button", { name: "Save changes" }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await expect(page.getByText("Converged", { exact: true })).toBeVisible();
    expect(writes).toBe(1);
  });
}

test("label edit keeps its draft and original ETag when the detail refreshes before a conflict", async ({
  page,
}) => {
  await mockApi(page);
  await page.goto("/fleets/linux-build");
  await page.getByRole("button", { name: "Edit fleet" }).click();
  await page.getByRole("button", { name: "Advanced settings" }).click();
  await page.getByLabel("Labels", { exact: true }).fill("arm64");
  await page.route("**/api/v1/fleets/linux-build", (route) => {
    if (route.request().method() === "GET") {
      const updated = fleet("linux-build", 3);
      updated.spec.github.labels = ["another-operator"];
      return route.fulfill({ headers: { etag: '"fleet-incarnation:3"' }, json: updated });
    }
    expect(route.request().headers()["if-match"]).toBe('"fleet-incarnation:2"');
    expect(route.request().postDataJSON().github.labels).toEqual(["arm64"]);
    return route.fulfill({ status: 412, json: { code: "PreconditionFailed", detail: "stale" } });
  });
  await page.waitForResponse(
    (response) =>
      response.url().endsWith("/fleets/linux-build") && response.request().method() === "GET",
    { timeout: 15_000 },
  );
  await page.getByRole("button", { name: "Save changes" }).click();
  await expect(page.getByRole("alert")).toContainText("changed since you opened it");
  await expect(page.getByLabel("Labels", { exact: true })).toHaveValue("arm64");
});

test("an empty desired label list shows its scale set name fallback", async ({ page }) => {
  await mockApi(page);
  const current = fleet("linux-build");
  current.spec.github.labels = [];
  await page.route("**/api/v1/fleets/linux-build", (route) =>
    route.fulfill({ headers: { etag: '"original:2"' }, json: current }),
  );
  await page.goto("/fleets/linux-build");
  await expect(page.getByText("linux-build (default)", { exact: true })).toBeVisible();
});
