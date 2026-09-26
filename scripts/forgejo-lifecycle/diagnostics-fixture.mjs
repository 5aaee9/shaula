// HTTPS transport only. The production daemon still owns browser OIDC sessions.
import { createServer } from "node:https";
import { request } from "node:http";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { Fixture } from "./fixture.mjs";
import { freePort } from "./support.mjs";

export class DiagnosticsFixture extends Fixture {
  async startDaemon() {
    if (!this.browserProxy) {
      const port = await freePort();
      this.publicUrl = `https://localhost:${port}`;
      this.browserProxy = createServer({ cert: await readFile(this.oidc.certificate),
        key: await readFile(join(this.directory, "oidc.key")) }, (req, res) => {
        const upstream = request({ hostname: "127.0.0.1", port: this.port, path: req.url,
          method: req.method, headers: req.headers }, response => {
          res.writeHead(response.statusCode, response.headers); response.pipe(res);
        });
        upstream.on("error", () => { if (!res.headersSent) res.writeHead(502); res.end(); });
        req.pipe(upstream);
      });
      await new Promise((resolve, reject) => { this.browserProxy.once("error", reject); this.browserProxy.listen(port, "127.0.0.1", resolve); });
    }
    await super.startDaemon();
  }
  async close() {
    try { await super.close(); }
    finally { if (this.browserProxy) await new Promise(resolve => { this.browserProxy.closeAllConnections(); this.browserProxy.close(resolve); }); }
  }
}
