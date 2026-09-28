import { expect, test } from "@playwright/test";
import { mockApi } from "./fixtures";

const fleetsList = (url: URL) => url.pathname === "/api/v1/fleets";

test("fleet inventory follows every list cursor", async ({ page }) => {
  await mockApi(page);
  const cursors: (string | null)[] = [];
  await page.route(fleetsList, (route) => {
    const cursor = new URL(route.request().url()).searchParams.get("cursor");
    cursors.push(cursor);
    const summary = (key: string) => ({ key, revision: 2, incarnation: "fleet-incarnation" });
    return route.fulfill({
      json:
        cursor === null
          ? { fleets: [summary("linux-build")], next_cursor: "page-2" }
          : { fleets: [summary("release-runners")], next_cursor: null },
    });
  });
  await page.goto("/fleets");
  await expect(page.getByRole("link", { name: "Open linux-build" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Open release-runners" })).toBeVisible();
  expect(cursors).toContain(null);
  expect(cursors).toContain("page-2");
});

test("a repeating list cursor is reported instead of looping", async ({ page }) => {
  await mockApi(page);
  let requests = 0;
  await page.route(fleetsList, (route) => {
    requests += 1;
    return route.fulfill({
      json: {
        fleets: [{ key: "linux-build", revision: 2, incarnation: "fleet-incarnation" }],
        next_cursor: "same",
      },
    });
  });
  await page.goto("/fleets");
  await expect(page.getByText("The server repeated a list cursor.")).toBeVisible();
  expect(requests).toBeLessThan(10);
});
