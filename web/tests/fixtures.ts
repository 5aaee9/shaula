import type { Page } from "@playwright/test";

export const scopes = [
  "fleet.read",
  "fleet.write",
  "fleet.retire",
  "template.read",
  "template.publish",
  "template.retire",
  "auth.read",
  "auth.write",
  "auth.retire",
];
export const fleetKeys = ["linux-build", "release-runners", "integration-tests"];
export function fleet(key: string, revision = 2) {
  return {
    key,
    metadata: { incarnation: "fleet-incarnation", revision },
    spec: {
      github: {
        target: { kind: "organization", owner: "acme" },
        auth_profile_ref: "github-build",
        scale_set_name: key,
        runner_group: "Default",
        labels: ["linux", "x64"],
      },
      capacity: { min_runners: 1, max_runners: 10 },
      template_profile_ref: { key: "kubernetes-linux", revision: 3 },
      template_inputs: {},
    },
    resolved: {
      template: {
        key: "kubernetes-linux",
        revision: 3,
        artifactDigest: `sha256:${"a".repeat(64)}`,
        attestationId: "verified",
      },
      authDesired: { profileKey: "github-build", revision: 1 },
    },
  };
}

export async function mockApi(page: Page, permissions = scopes) {
  await page.route("**/readyz", (route) => route.fulfill({ json: { ready: true } }));
  await page.route("**/api/v1/**", async (route) => {
    const path = new URL(route.request().url()).pathname.replace("/api/v1", "");
    const key = path.split("/")[2];
    const headers = { etag: '"fleet-incarnation:2"' };
    if (path === "/session")
      return route.fulfill({
        headers: { "x-csrf-token": "test-session-csrf" },
        json: { name: "Alex Morgan", scopes: permissions },
      });
    if (path === "/fleets")
      return route.fulfill({
        json: {
          fleets: fleetKeys.map((key) => ({ key, revision: 2, incarnation: "fleet-incarnation" })),
        },
      });
    if (path.startsWith("/fleets/") && path.endsWith("/status"))
      return route.fulfill({
        json: {
          fleetKey: key,
          desiredRevision: 2,
          observedRevision: 2,
          phase: key === "integration-tests" ? "Blocked" : "Ready",
          capacity: {
            effective: key === "linux-build" ? 6 : 2,
            assignedDemand: 3,
            occupancy: 6,
            target: 8,
          },
          conditions: [{ type: "TemplateReady", status: true, reason: null }],
          lastError: null,
        },
      });
    if (path.startsWith("/fleets/")) return route.fulfill({ headers, json: fleet(key) });
    if (path === "/template-profiles")
      return route.fulfill({
        json: {
          profiles: [
            {
              key: "kubernetes-linux",
              incarnation: "template-inc",
              desiredRevision: 3,
              activeRevision: 3,
              status: "Active",
            },
          ],
        },
      });
    if (path.startsWith("/template-profiles/"))
      return route.fulfill({
        headers: { etag: '"template-inc:3"' },
        json: {
          key,
          incarnation: "template-inc",
          desiredRevision: 3,
          activeRevision: 3,
          status: "Active",
          platform: "kubernetes",
          bindingsContract: "shaula.bindings.kubernetes/v1",
          bindings_present: true,
          artifactDigest: `sha256:${"a".repeat(64)}`,
          engineRef: "terraform",
          state: "Active",
        },
      });
    if (path.startsWith("/github-auth-profiles/"))
      return route.fulfill({
        headers: { etag: '"auth-inc:1"' },
        json: {
          key,
          incarnation: "auth-inc",
          desiredRevision: 1,
          activeRevision: 1,
          status: "Active",
          kind: "pat",
          identity: "build-bot",
          credential_present: true,
          target_allowlist: ["acme"],
        },
      });
    if (/\/(fleet|profile)-changes\//.test(path))
      return route.fulfill({
        json: {
          id: key,
          resourceKey: "linux-build",
          revision: 3,
          state: "Converged",
          kind: "Replace",
          reason: null,
        },
      });
    return route.fulfill({ status: 404, json: { code: "NotFound", detail: "Missing test route" } });
  });
}
