import { expect, test } from "@playwright/test";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import path from "node:path";

const execute = promisify(execFile);
test("real daemon OIDC browser issuance authorizes CLI and revocation takes effect", async ({
  page,
  request,
}) => {
  await request.post("/__test/provider", { data: {} });
  await page.goto("/settings/access-tokens");
  await expect(page.getByRole("heading", { name: "Access tokens", exact: true })).toBeVisible();
  await page.getByLabel("Name", { exact: true }).fill("Browser CLI acceptance");
  await page.getByRole("checkbox", { name: "fleet.read", exact: true }).check();
  await page.getByRole("button", { name: "Create token", exact: true }).click();
  await page.getByRole("button", { name: "Reveal token" }).click();
  const credential = await page.locator("code").filter({ hasText: "shaula_pat_v1_" }).innerText();
  expect(credential.startsWith("shaula_pat_v1_")).toBe(true);
  const fixture = await (await request.get("/__test/provider")).json();
  const binary = path.resolve(
    "../target/debug",
    process.platform === "win32" ? "shaula.exe" : "shaula",
  );
  const environment = { ...process.env, SHAULA_ACCESS_TOKEN: credential };
  delete environment.SHAULA_CONTEXT;
  const flags = ["--server", fixture.cliOrigin, "--allow-loopback-http", "--output", "json"];
  const output = await execute(binary, [...flags, "fleets", "list"], { env: environment });
  const envelope = JSON.parse(output.stdout);
  expect(envelope.error).toBeNull();
  expect(Array.isArray(envelope.data.fleets)).toBe(true);
  expect(output.stdout.includes(credential)).toBe(false);
  await execute(binary, [...flags, "tokens", "revoke", "--current", "--yes"], { env: environment });
  const rejected = await execute(binary, [...flags, "auth", "whoami"], { env: environment }).then(
    () => ({ code: 0, stdout: "{}" }),
    (error: { code: number; stdout: string }) => ({ code: error.code, stdout: error.stdout }),
  );
  expect(rejected.code).toBe(3);
  expect(JSON.parse(rejected.stdout).error.http_status).toBe(401);
  await page.reload();
  await expect(
    page.getByRole("heading", { name: "Browser CLI acceptance (revoked)", exact: true }),
  ).toBeVisible();
  expect(await page.locator("code").filter({ hasText: "shaula_pat_v1_" }).count()).toBe(0);
});
