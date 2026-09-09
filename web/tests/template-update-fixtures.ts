import type { Page } from "@playwright/test";
import { mockApi, profile, scopes } from "./fixtures";
import { dockerDigest, sources, variables } from "./template-library-fixtures";

export const currentDigest = `sha256:${"a".repeat(64)}`;
export const updatePath = "/templates/custom-docker/update";
export const updateApi = "**/api/v1/template-profiles/custom-docker/updates";
export const updateSources = [
  ...sources,
  { ...sources[0], key: "docker-alternative", engineRef: "opentofu" },
];

export async function mockTemplateUpdate(
  page: Page,
  options: { permissions?: string[]; status?: string; digest?: string; version?: string } = {},
) {
  await mockApi(page, options.permissions || scopes);
  const resource = {
    ...profile("custom-docker", 3, options.status || "Active"),
    platform: "docker",
    bindingsContract: "shaula.bindings.docker/v1",
    bindings_present: true,
  };
  await page.route("**/api/v1/template-profiles", (route) =>
    route.fulfill({ json: { profiles: [resource] } }),
  );
  await page.route("**/api/v1/template-profiles/custom-docker", (route) =>
    route.fulfill({ headers: { etag: options.version || '"template-inc:3"' }, json: resource }),
  );
  await page.route("**/api/v1/template-profiles/custom-docker/revisions/3", (route) =>
    route.fulfill({
      json: {
        revision: 3,
        artifactDigest: options.digest || currentDigest,
        engineRef: "terraform",
        platform: "docker",
        state: "Active",
        reason: null,
      },
    }),
  );
  await page.route("**/api/v1/template-sources", (route) =>
    route.fulfill({ json: { sources: updateSources } }),
  );
  await page.route("**/api/v1/template-artifacts/*/variables", (route) =>
    route.fulfill({
      json: {
        ...variables,
        artifactDigest: dockerDigest,
        parameters: [
          ...variables.parameters,
          {
            ...variables.parameters[0],
            key: "quota",
            label: "Quota",
            typeName: "number",
            options: [{ valueJson: "9007199254740993" }],
          },
          {
            ...variables.parameters[0],
            key: "secret",
            label: "Secret parameter",
            sensitive: true,
            options: [{ valueJson: '"never-display-this"' }],
          },
        ],
      },
    }),
  );
}
