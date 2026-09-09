import { expect, test } from "@playwright/test";
import { dockerDigest } from "./template-library-fixtures";
import { mockTemplateUpdate, updateApi, updatePath } from "./template-update-fixtures";

for (const operation of ["update", "publication"]) {
  test(`a 200 NoOp ${operation} returns to the template without inventing a Change`, async ({
    page,
  }) => {
    await page.clock.install();
    await mockTemplateUpdate(page, { digest: dockerDigest, sourceKey: "docker" });
    let changeReads = 0;
    let writes = 0;
    await page.route("**/api/v1/profile-changes/**", (route) => {
      changeReads++;
      return route.fulfill({ status: 404, json: { code: "NotFound" } });
    });
    await page.route(
      operation === "update" ? updateApi : "**/api/v1/template-profiles/custom-docker",
      (route) => {
        if (route.request().method() === "GET") return route.fallback();
        writes++;
        expect(route.request().headers()["if-match"]).toBe('"template-inc:3"');
        const body = route.request().postDataJSON();
        expect(body.source_key).toBe("docker");
        expect(body.artifact_digest).toBe(dockerDigest);
        expect(body.engine_ref).toBe("terraform");
        expect(body.fleet_input_policy.runner_image).toEqual(["runner:stable", "runner:canary"]);
        return route.fulfill({
          status: 200,
          headers: { etag: '"template-inc:3"' },
          json: { changeId: "", state: "Converged", revision: 3, noOp: true },
        });
      },
    );
    await page.goto(operation === "update" ? updatePath : "/templates/custom-docker/revisions/new");
    if (operation === "publication") {
      await page.getByRole("radio", { name: "Default template", exact: true }).check();
      await page
        .getByRole("combobox", { name: "Default template", exact: true })
        .selectOption("docker");
    }
    await page.getByRole("button", { name: "Use declared options" }).click();
    await page
      .getByRole("button", { name: operation === "update" ? "Update" : "Publish", exact: true })
      .click();
    await expect(page).toHaveURL(/\/templates\?key=custom-docker$/);
    await expect(
      page.getByRole("status").filter({ hasText: "No changes were needed." }),
    ).toBeVisible();
    await expect(page.getByText("Change for custom-docker", { exact: true })).toHaveCount(0);
    await expect(page.getByRole("link", { name: "View change", exact: true })).toHaveCount(0);
    await expect(page.getByLabel("Template revision", { exact: true })).toHaveValue("3");
    await page.clock.fastForward(10_000);
    expect(changeReads).toBe(0);
    expect(writes).toBe(1);
  });
}
