import { defineConfig, devices } from "@playwright/test";
export default defineConfig({
  testDir: "./e2e",
  workers: 1,
  use: {
    baseURL: "https://localhost:5181",
    ignoreHTTPSErrors: true,
    ...devices["Desktop Chrome"],
    screenshot: "only-on-failure",
  },
  webServer: {
    command: "cargo test -p shaula --test oidc_browser -- --ignored --nocapture",
    cwd: "..",
    url: "https://localhost:5181/auth/oidc/callback",
    ignoreHTTPSErrors: true,
    timeout: 120_000,
    reuseExistingServer: false,
  },
});
