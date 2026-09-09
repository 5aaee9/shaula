import { expect, test } from "@playwright/test";
import { V2_PROFILE } from "./auth-fixtures";
import { mockApi } from "./fixtures";

for (const canonicalVersion of [true, false]) {
  test(`compressed auth rotation ${canonicalVersion ? "uses the canonical version" : "rejects a weak-only validator"}`, async ({
    page,
  }) => {
    await mockApi(page);
    let writes = 0;
    await page.route("**/api/v1/github-auth-profiles/shared-github", async (route) => {
      if (route.request().method() === "GET") {
        return route.fulfill({
          headers: {
            etag: 'W/"auth-inc:1"',
            ...(canonicalVersion ? { "shaula-resource-version": '"auth-inc:1"' } : {}),
          },
          json: V2_PROFILE,
        });
      }
      writes++;
      if (route.request().headers()["if-match"] !== '"auth-inc:1"') {
        return route.fulfill({ status: 412, json: { code: "PreconditionFailed" } });
      }
      return route.fulfill({
        status: 202,
        json: { changeId: "c3", state: "Pending", revision: 2 },
      });
    });
    await page.goto("/auth?key=shared-github");
    await page.getByRole("button", { name: "Rotate credential" }).click();
    await page.getByLabel("Private key (PEM)").fill("-----BEGIN TEST-----");
    await page.getByRole("dialog").getByRole("button", { name: "Rotate credential" }).click();
    if (canonicalVersion) {
      await expect(page.getByRole("dialog")).toHaveCount(0);
      expect(writes).toBe(1);
    } else {
      await expect(page.getByRole("alert")).toContainText("resource version is missing");
      expect(writes).toBe(0);
    }
  });
}
