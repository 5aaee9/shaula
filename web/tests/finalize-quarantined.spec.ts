import { expect, test } from "@playwright/test";
import { mockApi } from "./fixtures";

test("finalize quarantined runs bounded lanes, reports partial failure and retries the rest", async ({
  page,
}, testInfo) => {
  await mockApi(page);
  let remaining = ["g1", "g2", "g3", "g4", "g5", "g6"];
  let failOnce = new Set(["g3"]);
  let inFlight = 0;
  let maxInFlight = 0;
  const log: { id: string; key: string; status: number }[] = [];

  await page.route("**/api/v1/generations?*", (route) =>
    route.fulfill({ json: { items: remaining.map((id) => ({ id })) } }),
  );
  await page.route("**/api/v1/generations/*/finalize", async (route) => {
    const id = new URL(route.request().url()).pathname.split("/").at(-2)!;
    const key = route.request().headers()["idempotency-key"];
    expect(route.request().postDataJSON()).toEqual({ reason: "VMs deleted in console" });
    inFlight += 1;
    maxInFlight = Math.max(maxInFlight, inFlight);
    await new Promise((resolve) => setTimeout(resolve, 150));
    inFlight -= 1;
    if (failOnce.delete(id)) {
      log.push({ id, key, status: 503 });
      return route.fulfill({ status: 503, json: { code: "Unavailable", detail: "busy" } });
    }
    remaining = remaining.filter((other) => other !== id);
    log.push({ id, key, status: 202 });
    return route.fulfill({ status: 202, json: { changeId: `finalize-${id}`, state: "Accepted" } });
  });

  await page.goto("/fleets/linux-build");
  await page.getByRole("button", { name: "Finalize quarantined" }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("6 quarantined runners will be marked Destroyed.");
  await dialog.getByLabel("Verification").fill("VMs deleted in console");
  await dialog.getByRole("button", { name: "Finalize all" }).click();
  await expect(dialog).toContainText("1 of 6 runners could not be finalized. Retry to finish.");

  expect(maxInFlight).toBeGreaterThan(1);
  expect(maxInFlight).toBeLessThanOrEqual(4);
  expect(log.map((entry) => entry.id).sort()).toEqual(["g1", "g2", "g3", "g4", "g5", "g6"]);
  const firstAttempt = log[0].key.slice(0, -"-g1".length);
  for (const entry of log) expect(entry.key).toBe(`${firstAttempt}-${entry.id}`);

  // The list refetches to the one survivor; the retry uses a fresh attempt key.
  await expect(dialog).toContainText("1 quarantined runner will be marked Destroyed.");
  await dialog.getByRole("button", { name: "Finalize all" }).click();
  await expect(dialog).toHaveCount(0);
  const retry = log.slice(6);
  expect(retry.map((entry) => entry.id)).toEqual(["g3"]);
  expect(retry[0].status).toBe(202);
  expect(retry[0].key.startsWith(`${firstAttempt}-`)).toBe(false);
  expect(retry[0].key.endsWith("-g3")).toBe(true);
  failOnce = new Set();

  await testInfo.attach("finalize-requests.json", {
    body: JSON.stringify({ maxInFlight, requests: log }, null, 2),
    contentType: "application/json",
  });
});
