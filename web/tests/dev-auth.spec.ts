import { createServer as httpServer } from "node:http";
import type { AddressInfo } from "node:net";
import { expect, test } from "@playwright/test";
import { createServer, preview } from "vite";

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
      const response = await fetch(`${origin}/fleets`, { redirect: "manual" });
      expect(response.status).toBe(302);
      expect(await response.text()).toBe("");
      expect((await fetch(`${origin}${asset}`)).status).toBe(401);
      expect((await fetch(`${origin}/fleets`, { method: "HEAD" })).status).toBe(401);
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
