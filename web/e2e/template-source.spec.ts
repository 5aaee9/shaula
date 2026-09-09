import { expect, test } from "@playwright/test";

test("published source survives real database reads and an explicit source-only update", async ({
  page,
  request,
}, testInfo) => {
  // Production reconciliation runs every 15 seconds; all three publications
  // must progress through that loop rather than a test-only activation shortcut.
  test.setTimeout(90_000);
  await request.post("/__test/provider", { data: {} });
  await page.goto("/templates");
  await page.getByRole("button", { name: "Use template docker", exact: true }).click();
  const form = page.getByRole("form", { name: "Template configuration" });
  await form.getByLabel("Profile key", { exact: true }).fill("browser-source");
  await form.getByRole("button", { name: "Use defaults", exact: true }).click();
  await form.getByRole("button", { name: "Use declared options", exact: true }).click();
  const profilePath = "/api/v1/template-profiles/browser-source";
  const publishing = page.waitForResponse(
    (response) => response.url().endsWith(profilePath) && response.request().method() === "PUT",
  );
  await form.getByRole("button", { name: "Publish", exact: true }).click();
  const published = await publishing;
  expect(published.status()).toBe(202);
  const initial = published.request().postDataJSON();
  expect(initial).toMatchObject({ source_key: "docker", engine_ref: "terraform" });
  await expect(page).toHaveURL(/\/templates\?key=browser-source$/);

  const revision = async (number: number) => {
    const response = await page.request.get(`${profilePath}/revisions/${number}`);
    expect(response.status()).toBe(200);
    return response.json();
  };
  await expect.poll(async () => (await revision(1)).state, { timeout: 25_000 }).toBe("Active");
  expect(await revision(1)).toMatchObject({ sourceKey: "docker" });

  // A regular artifact publication with no source must not inherit provenance,
  // even when it reuses exactly the same executable content and configuration.
  const head = await page.request.get(profilePath);
  const session = await page.request.get("/api/v1/session");
  const { source_key: sourceKey, ...artifactPublication } = initial;
  expect(sourceKey).toBe("docker");
  const unassociated = await page.request.put(profilePath, {
    headers: {
      origin: "https://localhost:5181",
      "x-csrf-token": session.headers()["x-csrf-token"],
      "if-match": head.headers()["shaula-resource-version"] || head.headers().etag,
      "idempotency-key": "browser-clear-source",
    },
    data: artifactPublication,
  });
  expect(unassociated.status()).toBe(202);
  await expect.poll(async () => (await revision(2)).state, { timeout: 25_000 }).toBe("Active");
  expect(await revision(2)).toMatchObject({ sourceKey: null });
  expect(await revision(1)).toMatchObject({ sourceKey: "docker" });

  await page.goto("/templates/browser-source/update");
  const review = page.getByRole("form", { name: "Template update review" });
  await expect(review.getByLabel("Default template", { exact: true })).toHaveValue("docker");
  const update = review.getByRole("button", { name: "Update", exact: true });
  await expect(update).toBeEnabled();
  const updating = page.waitForResponse(
    (response) =>
      response.url().endsWith(`${profilePath}/updates`) && response.request().method() === "POST",
  );
  await update.click();
  const updated = await updating;
  expect(updated.status()).toBe(202);
  expect(updated.request().postDataJSON()).toEqual({
    source_key: "docker",
    artifact_digest: initial.artifact_digest,
    engine_ref: initial.engine_ref,
  });
  await expect.poll(async () => (await revision(3)).state, { timeout: 25_000 }).toBe("Active");
  expect(await revision(3)).toMatchObject({
    sourceKey: "docker",
    artifactDigest: initial.artifact_digest,
  });
  expect(await revision(2)).toMatchObject({ sourceKey: null });

  // Full navigation discards the publication draft; the saved API relation now
  // selects the source without a dropdown or any caller-supplied route state.
  await page.goto("/templates/browser-source/update");
  await expect(review.getByRole("combobox", { name: "Default template" })).toHaveCount(0);
  await expect(review.getByRole("region", { name: "Update source" })).toContainText("docker");
  await expect(
    review.getByRole("region", { name: "Target revision" }).getByRole("status"),
  ).toContainText("Up to date");
  await expect(review.getByRole("button", { name: "Update", exact: true })).toBeDisabled();
  await expect(review.getByRole("button", { name: "Use declared options" })).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath("saved-default-source.png"), fullPage: true });

  // Explicitly approving the identical policy is a real 200 NoOp, without a
  // fourth revision or a nonexistent Change to poll on the destination page.
  await review.getByRole("button", { name: "Use declared options" }).click();
  const unchanged = page.waitForResponse(
    (response) =>
      response.url().endsWith(`${profilePath}/updates`) && response.request().method() === "POST",
  );
  await review.getByRole("button", { name: "Update", exact: true }).click();
  const unchangedResponse = await unchanged;
  expect(unchangedResponse.status()).toBe(200);
  expect(await unchangedResponse.json()).toMatchObject({ noOp: true, changeId: "" });
  await expect(page.getByText("No changes were needed.", { exact: true })).toBeVisible();
  expect(await (await page.request.get(profilePath)).json()).toMatchObject({ desiredRevision: 3 });
});
