import { createServer as httpServer, request as httpRequest } from "node:http";
import type { AddressInfo } from "node:net";
import { expect, test } from "@playwright/test";
import { createServer, preview } from "vite";

async function rawStatus(origin: string, path: string) {
  // Preserve dot segments so the guard is checked against the actual request path.
  return new Promise<number | undefined>((resolve, reject) => {
    const request = httpRequest(origin, { path }, (response) => {
      response.resume();
      response.on("end", () => resolve(response.statusCode));
      response.on("error", reject);
    });
    request.on("error", reject);
    request.end();
  });
}

test("normal Vite dev and preview authenticate source assets and disable shared caching", async () => {
  const backend = httpServer((req, res) => {
    res.writeHead(req.headers.cookie === "__Host-shaula-session=test" ? 200 : 401, {
      "Cache-Control": "private, no-store",
    });
    res.end();
  });
  await new Promise<void>((resolve) => backend.listen(0, "127.0.0.1", resolve));
  const previous = process.env.SHAULA_API_TARGET;
  process.env.SHAULA_API_TARGET = `http://127.0.0.1:${(backend.address() as AddressInfo).port}`;
  const dev = await createServer({
    configFile: "vite.config.ts",
    server: { host: "127.0.0.1", port: 0 },
  });
  const production = await preview({
    configFile: "vite.config.ts",
    preview: { host: "127.0.0.1", port: 0 },
  });
  try {
    await dev.listen();
    expect(dev.config.server.ws).toBe(false);
    for (const [server, asset] of [
      [dev.httpServer!, "/src/app.tsx"],
      [production.httpServer, "/index.html"],
    ] as const) {
      const origin = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
      for (const path of [
        "/fleets",
        "/templates/new",
        "/templates/linux-build/revisions/new",
        "/templates/linux-x64.1_a/revisions/new",
      ]) {
        const response = await fetch(`${origin}${path}`, { redirect: "manual" });
        expect(response.status).toBe(302);
        expect(response.headers.get("location")).toBe(
          `/auth/oidc/login?${new URLSearchParams({ return_to: path })}`,
        );
        expect(await response.text()).toBe("");
        expect((await fetch(`${origin}${path}`, { method: "HEAD" })).status).toBe(401);
      }
      for (const path of [
        "/templates/linux-build",
        "/templates/new/extra",
        "/templates//revisions/new",
        "/templates/./revisions/new",
        "/templates/../revisions/new",
        "/templates/linux-build/../new",
        "/templates/%2e%2e/revisions/new",
        "/templates/with%20space/revisions/new",
        "/templates/a/b/revisions/new",
        "/templates/linux-build/revisions/new/",
        `/templates/${"a".repeat(129)}/revisions/new`,
      ]) {
        expect(await rawStatus(origin, path), path).toBe(401);
      }
      expect((await fetch(`${origin}${asset}`)).status).toBe(401);
      const authenticated = await fetch(`${origin}${asset}`, {
        headers: { cookie: "__Host-shaula-session=test" },
      });
      expect(authenticated.status).toBe(200);
      expect(authenticated.headers.get("cache-control")).toBe("private, no-store");
      await authenticated.body?.cancel();
    }
  } finally {
    // Finish eager import transforms before closing their optimizer and plugin container.
    await dev.environments.client.waitForRequestsIdle();
    await dev.close();
    await new Promise<void>((resolve) => production.httpServer.close(() => resolve()));
    await new Promise<void>((resolve) => backend.close(() => resolve()));
    if (previous === undefined) delete process.env.SHAULA_API_TARGET;
    else process.env.SHAULA_API_TARGET = previous;
  }
});
