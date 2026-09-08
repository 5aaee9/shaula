import { expect, test } from "@playwright/test";

test("real database imports default templates and supplies Terraform variables to publishing", async ({
  page,
  request,
}) => {
  await request.post("/__test/provider", { data: {} });
  expect((await request.get("/api/v1/template-sources")).status()).toBe(401);
  await page.goto("/templates");
  await expect(page.getByRole("heading", { name: "Templates", exact: true })).toBeVisible();
  const catalog = await page.request.get("/api/v1/template-sources");
  expect(catalog.status()).toBe(200);
  expect(catalog.headers()["cache-control"]).toBe("private, no-store");
  const { sources } = await catalog.json();
  expect(sources.map((source: { key: string }) => source.key)).toEqual(["docker", "kubernetes"]);
  const docker = sources[0];
  expect(docker.artifactDigest).toMatch(/^sha256:[a-f0-9]{64}$/);
  const variables = await page.request.get(
    `/api/v1/template-artifacts/${docker.artifactDigest}/variables`,
  );
  expect(variables.status()).toBe(200);
  expect(variables.headers()["cache-control"]).toBe("private, no-store");
  const definitions = await variables.json();
  expect(definitions).toMatchObject({ artifactDigest: docker.artifactDigest, available: true });
  expect(definitions.bindings).toEqual(
    expect.arrayContaining([
      expect.objectContaining({
        key: "docker_host",
        typeName: "string",
        defaultValueJson: '"unix:///var/run/docker.sock"',
      }),
      expect.objectContaining({ key: "registry_auth", sensitive: true }),
    ]),
  );
  expect(
    definitions.bindings.find((field: { key: string }) => field.key === "registry_auth"),
  ).not.toHaveProperty("defaultValueJson");
  expect((await page.request.get("/api/v1/template-profiles/docker")).status()).toBe(404);

  await page.getByRole("button", { name: "Use template docker", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByLabel("Default template", { exact: true })).toHaveValue("docker");
  const host = dialog.getByLabel("Docker host", { exact: true });
  await expect(host).toBeVisible();
  await expect(host).toHaveValue("");
  await expect(dialog.getByLabel("Registry auth", { exact: true })).toHaveAttribute(
    "type",
    "password",
  );
  await expect(dialog.getByRole("region", { name: "Fleet input variables" })).toBeVisible();
  await dialog.getByRole("button", { name: "Use defaults", exact: true }).click();
  await expect(host).toHaveValue("unix:///var/run/docker.sock");
  await host.fill("unix:///run/custom-docker.sock");
  await dialog.getByRole("button", { name: "Use defaults", exact: true }).click();
  await expect(host).toHaveValue("unix:///run/custom-docker.sock");
  await dialog.getByRole("button", { name: "Advanced settings", exact: true }).click();
  await expect(dialog.getByLabel("Fleet input policy (JSON)", { exact: true })).toHaveValue("{}");
  await dialog.getByRole("button", { name: "Use declared options", exact: true }).click();
  const policy = JSON.parse(
    await dialog.getByLabel("Fleet input policy (JSON)", { exact: true }).inputValue(),
  );
  expect(policy.runner_image).toEqual(["localhost:5001/shaula-runner:2.337.0-bootstrap-v1"]);
  await expect(dialog.getByLabel("Bindings (JSON)", { exact: true })).toHaveValue(
    '{"docker_host":"unix:///run/custom-docker.sock"}',
  );
  await dialog.getByRole("button", { name: "Advanced settings", exact: true }).click();
  await expect(host).toBeVisible();
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  expect((await page.request.get("/api/v1/template-profiles/docker")).status()).toBe(404);
});
