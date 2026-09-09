import { expect, test } from "@playwright/test";
import { mockApi } from "./fixtures";
import { mockTemplateLibrary } from "./template-library-fixtures";

test("canceling the template page returns to the list and a new visit starts a clean draft", async ({
  page,
}) => {
  await mockTemplateLibrary(page);
  await page.goto("/templates");
  await page.getByRole("button", { name: "Publish template", exact: true }).click();
  await expect(page).toHaveURL(/\/templates\/new$/);
  await expect(page.getByRole("heading", { name: "Publish template", exact: true })).toBeVisible();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  const form = page.getByRole("form", { name: "Template configuration" });
  await form.getByLabel("Profile key").fill("discarded-draft");
  await form.getByLabel("Template archive (.tar.gz)").setInputFiles({
    name: "discarded.tar.gz",
    mimeType: "application/gzip",
    buffer: Buffer.from("discarded-template-archive"),
  });
  await form.getByRole("button", { name: "Advanced settings" }).click();
  await form.getByLabel("Bindings (JSON)").fill('{"docker_host":"private-draft"}');
  await form.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(page).toHaveURL(/\/templates(?:\?[^#]*)?$/);
  await expect(form).toHaveCount(0);
  await expect(page.getByRole("heading", { name: "Templates", exact: true })).toBeVisible();

  await page.getByRole("button", { name: "Publish template", exact: true }).click();
  await expect(page).toHaveURL(/\/templates\/new$/);
  await expect(form.getByLabel("Profile key")).toHaveValue("");
  await expect(form.getByLabel("Template archive (.tar.gz)")).toHaveValue("");
  await expect(form.getByRole("radio", { name: "Upload archive", exact: true })).toBeChecked();
  await form.getByRole("button", { name: "Advanced settings" }).click();
  await expect(form.getByLabel("Bindings (JSON)")).toHaveValue("{}");
  await page.reload();
  await expect(page.getByRole("heading", { name: "Publish template", exact: true })).toBeVisible();
  await expect(form.getByLabel("Profile key")).toHaveValue("");
});

test("revision pages support direct navigation and retain the publication precondition", async ({
  page,
}) => {
  await mockApi(page);
  await page.goto("/templates?key=kubernetes-linux");
  await page.getByRole("button", { name: "New revision", exact: true }).click();
  await expect(page).toHaveURL(/\/templates\/kubernetes-linux\/revisions\/new$/);
  await page.reload();
  await expect(
    page.getByRole("heading", { name: "Publish revision of kubernetes-linux", exact: true }),
  ).toBeVisible();
  const form = page.getByRole("form", { name: "Template configuration" });
  await expect(form.getByLabel("Profile key")).toHaveValue("kubernetes-linux");
  await expect(form.getByLabel("Profile key")).toBeDisabled();
  await form.getByRole("radio", { name: "Existing artifact", exact: true }).check();
  const digest = `sha256:${"a".repeat(64)}`;
  await form.getByLabel("Existing artifact digest").fill(digest);
  let publications = 0;
  await page.route("**/api/v1/template-profiles/kubernetes-linux", (route) => {
    if (route.request().method() === "GET") return route.fallback();
    publications++;
    expect(route.request().method()).toBe("PUT");
    expect(route.request().headers()["if-match"]).toBe('"template-inc:3"');
    expect(route.request().headers()["if-none-match"]).toBeUndefined();
    expect(route.request().postDataJSON().artifact_digest).toBe(digest);
    return route.fulfill({
      status: 202,
      json: { changeId: "new-revision", state: "Accepted", revision: 4 },
    });
  });
  await form.getByRole("button", { name: "Publish", exact: true }).click();
  await expect(page).toHaveURL(/\/templates(?:\?[^#]*)?$/);
  await expect(form).toHaveCount(0);
  await expect(page.getByText("Change for kubernetes-linux")).toBeVisible();
  expect(publications).toBe(1);
});

test("finishing a publication after leaving the page preserves the next template draft", async ({
  page,
}) => {
  await mockTemplateLibrary(page);
  let release!: () => void;
  const pendingPublication = new Promise<void>((resolve) => {
    release = resolve;
  });
  await page.route("**/api/v1/template-profiles/first-template", async (route) => {
    if (route.request().method() === "GET") return route.fallback();
    expect(route.request().method()).toBe("PUT");
    await pendingPublication;
    await route.fulfill({
      status: 202,
      json: { changeId: "late-publication", state: "Accepted", revision: 1 },
    });
  });
  await page.goto("/templates/new");
  const form = page.getByRole("form", { name: "Template configuration" });
  await form.getByLabel("Profile key").fill("first-template");
  await form.getByRole("radio", { name: "Existing artifact", exact: true }).check();
  await form.getByLabel("Existing artifact digest").fill(`sha256:${"a".repeat(64)}`);
  const submitted = page.waitForRequest(
    (request) =>
      request.url().endsWith("/template-profiles/first-template") && request.method() === "PUT",
  );
  await form.getByRole("button", { name: "Publish", exact: true }).click();
  await submitted;
  await page.getByRole("link", { name: "Back to templates", exact: true }).click();
  await expect(page).toHaveURL(/\/templates$/);
  await page.getByRole("button", { name: "Publish template", exact: true }).click();
  await form.getByLabel("Profile key").fill("second-template");
  await form.getByRole("button", { name: "Advanced settings" }).click();
  await form.getByLabel("Bindings (JSON)").fill('{"docker_host":"second-draft"}');

  // The completed write still refreshes shared data after its form has unmounted.
  const refresh = page.waitForResponse("**/api/v1/template-sources");
  release();
  await refresh;
  await expect(page).toHaveURL(/\/templates\/new$/);
  await expect(form.getByLabel("Profile key")).toHaveValue("second-template");
  await expect(form.getByLabel("Bindings (JSON)")).toHaveValue('{"docker_host":"second-draft"}');
  await expect(form.getByRole("button", { name: "Publish", exact: true })).toBeEnabled();
});
