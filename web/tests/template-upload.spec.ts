import { createHash } from "node:crypto";
import { expect, test, type Page } from "@playwright/test";
import { mockTemplateLibrary } from "./template-library-fixtures";

test("archive selection reports invalid formats and removing a file clears the draft", async ({
  page,
}) => {
  await mockTemplateLibrary(page);
  await page.goto("/templates");
  await page.getByRole("button", { name: "Publish template", exact: true }).click();
  const form = page.getByRole("form", { name: "Template configuration" });
  const archive = form.getByLabel("Template archive (.tar.gz)");
  await expect(form.getByRole("group", { name: "Template source", exact: true })).toBeVisible();
  await expect(form.getByRole("radio", { name: "Upload archive", exact: true })).toBeChecked();
  await archive.setInputFiles({
    name: "runner.zip",
    mimeType: "application/zip",
    buffer: Buffer.from("invalid-template-format"),
  });
  await expect(form.getByRole("alert")).toContainText(".tar.gz");
  await expect(archive).toHaveValue("");
  await expect(form.getByRole("button", { name: "Remove archive", exact: true })).toHaveCount(0);

  await archive.setInputFiles({
    name: "runner.tar.gz",
    mimeType: "application/gzip",
    buffer: Buffer.from("selected-template-archive"),
  });
  await expect(form.getByRole("status").getByText("runner.tar.gz", { exact: true })).toBeVisible();
  await expect(form.getByRole("alert")).toHaveCount(0);
  await form.getByRole("button", { name: "Remove archive", exact: true }).click();
  await expect(archive).toHaveValue("");
  await expect(form.getByText("runner.tar.gz", { exact: true })).toHaveCount(0);
  await expect(form.getByRole("radio", { name: "Upload archive", exact: true })).toBeChecked();

  // Removing an archive also allows selecting the same file again.
  await archive.setInputFiles({
    name: "runner.tar.gz",
    mimeType: "application/gzip",
    buffer: Buffer.from("selected-template-archive"),
  });
  await expect(form.getByRole("status").getByText("runner.tar.gz", { exact: true })).toBeVisible();
});

test("dragged archives validate the format and can be inspected and published", async ({
  page,
}) => {
  await mockTemplateLibrary(page);
  const contents = "dropped-template-archive";
  const bytes = Buffer.from(contents);
  const digest = `sha256:${createHash("sha256").update(bytes).digest("hex")}`;
  let uploads = 0;
  await page.route("**/api/v1/template-artifacts/*", (route) => {
    uploads++;
    expect(route.request().method()).toBe("PUT");
    expect(route.request().postDataBuffer()).toEqual(bytes);
    return route.fulfill({ status: 201, json: { digest } });
  });
  await page.route("**/api/v1/template-profiles/dropped-archive", (route) => {
    if (route.request().method() === "GET") return route.fallback();
    expect(route.request().postDataJSON().artifact_digest).toBe(digest);
    return route.fulfill({
      status: 202,
      json: { changeId: "dropped-archive", state: "Accepted", revision: 1 },
    });
  });
  await page.goto("/templates");
  await page.getByRole("button", { name: "Publish template", exact: true }).click();
  await page.getByLabel("Profile key").fill("dropped-archive");
  await dropArchive(page, "runner.txt", contents);
  await expect(
    page.getByRole("form", { name: "Template configuration" }).getByRole("alert"),
  ).toContainText(".tar.gz");
  await expect(page.getByLabel("Template archive (.tar.gz)")).toHaveValue("");
  expect(uploads).toBe(0);

  await dropArchive(page, "runner.tgz", contents);
  await expect(
    page
      .getByRole("form", { name: "Template configuration" })
      .getByRole("status")
      .getByText("runner.tgz", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("form", { name: "Template configuration" }).getByRole("alert"),
  ).toHaveCount(0);
  await expect(page.getByLabel("Template archive (.tar.gz)")).toHaveValue(/runner\.tgz$/);
  await page.getByRole("button", { name: "Inspect variables", exact: true }).click();
  await expect(page.getByLabel("Docker host", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Publish", exact: true }).click();
  await expect(page).toHaveURL(/\/templates(?:\?[^#]*)?$/);
  await expect(page.getByRole("form", { name: "Template configuration" })).toHaveCount(0);
  expect(uploads).toBe(1);
});

async function dropArchive(page: Page, name: string, contents: string) {
  await page.getByLabel("Template archive (.tar.gz)").evaluate(
    (input, file) => {
      const dataTransfer = new DataTransfer();
      dataTransfer.items.add(new File([file.contents], file.name, { type: "application/gzip" }));
      input.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer }));
    },
    { name, contents },
  );
}
