import type { Page } from "@playwright/test";
import { mockApi } from "./fixtures";

export const dockerDigest = `sha256:${"d".repeat(64)}`;
export const kubernetesDigest = `sha256:${"e".repeat(64)}`;
export const sources = [
  { key: "docker", artifactDigest: dockerDigest, platform: "docker", engineRef: "terraform" },
  {
    key: "kubernetes",
    artifactDigest: kubernetesDigest,
    platform: "kubernetes",
    engineRef: "terraform",
  },
];
export const variables = {
  artifactDigest: dockerDigest,
  available: true,
  bindings: [
    {
      key: "docker_host",
      label: "Docker host",
      description: "Address of your Docker daemon.",
      typeName: "string",
      required: false,
      sensitive: false,
      defaultValueJson: '"unix:///var/run/docker.sock"',
      options: [],
    },
    {
      key: "quota",
      label: "Quota",
      typeName: "number",
      required: false,
      sensitive: false,
      defaultValueJson: "9007199254740993",
      options: [],
    },
    {
      key: "optional_name",
      label: "Optional name",
      typeName: "string",
      required: false,
      sensitive: false,
      defaultValueJson: "null",
      options: [],
    },
    {
      key: "token",
      label: "Access token",
      typeName: "string",
      required: false,
      sensitive: true,
      options: [],
    },
  ],
  parameters: [
    {
      key: "runner_image",
      label: "Runner image",
      typeName: "string",
      required: true,
      sensitive: false,
      defaultValueJson: '"runner:stable"',
      options: [{ valueJson: '"runner:stable"' }, { valueJson: '"runner:canary"' }],
    },
  ],
};

export async function mockTemplateLibrary(page: Page) {
  await mockApi(page);
  await page.route("**/api/v1/template-sources", (route) => route.fulfill({ json: { sources } }));
  await page.route("**/api/v1/template-artifacts/*/variables", (route) =>
    route.fulfill({
      json: {
        ...variables,
        artifactDigest: decodeURIComponent(new URL(route.request().url()).pathname.split("/")[4]),
      },
    }),
  );
}
