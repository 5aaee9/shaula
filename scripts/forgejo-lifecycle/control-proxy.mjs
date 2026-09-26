// Disposable Linux-host transport fault. The production binary is unchanged.
// A single UID+loopback-port OUTPUT rule routes worker requests through a root
// proxy; the root proxy's upstream connection is excluded by that UID match.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createServer, request } from "node:http";
import { readFile, writeFile, rename } from "node:fs/promises";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { randomUUID } from "node:crypto";
import { command, freePort, until } from "./support.mjs";

async function atomic(path, value) {
  await writeFile(`${path}.next`, JSON.stringify(value), { mode: 0o600 });
  await rename(`${path}.next`, path);
}

export async function serveProxy(port, upstream, config, onEvent = () => {}) {
  let current = { id: "none" };
  let held = [];
  let counts = { id: "none", matched: 0, committed: 0, pid: process.pid };
  // Root-owned event file is intentionally non-secret; only counters/PID are
  // emitted over stdout. The caller never reads credential-bearing traffic.
  const report = () => onEvent({ ...counts });
  const refresh = async () => {
    const next = JSON.parse(await readFile(config, "utf8"));
    if (next.id !== current.id) {
      for (const release of held.splice(0)) release();
      current = next;
      counts = { id: next.id, matched: 0, committed: 0, pid: process.pid };
      report();
    }
  };
  const timer = setInterval(() => { refresh().catch(() => {}); }, 100);
  const server = createServer(async (incoming, outgoing) => {
    try {
      await refresh();
      let body;
      let kind;
      let intent;
      if (new URL(incoming.url, "http://fixture.invalid").pathname.endsWith("/state")) kind = `state_${incoming.method.toLowerCase()}`;
      if (incoming.url.endsWith("/control")) {
        const chunks = [];
        let size = 0;
        for await (const chunk of incoming) {
          size += chunk.length;
          if (size > 65536) { outgoing.writeHead(413); outgoing.end(); return; }
          chunks.push(chunk);
        }
        body = Buffer.concat(chunks);
        const message = JSON.parse(body.toString("utf8")).message;
        kind = message.kind;
        intent = message.payload?.intent;
      }
      const selected = kind === current.kind && (!current.intent || intent === current.intent)
        && counts.matched < (current.limit ?? 1);
      const mode = selected ? current.mode : undefined;
      if (selected) { counts.matched++; report(); }
      if (mode === "reject_before") { incoming.resume(); outgoing.writeHead(503); outgoing.end(); return; }
      const forward = () => {
        const remote = request({ host: "127.0.0.1", port: upstream, path: incoming.url,
          method: incoming.method, headers: incoming.headers }, response => {
          if (selected && response.statusCode === 200) {
            if (mode === "hold_after") {
              const chunks = [];
              response.on("data", chunk => chunks.push(chunk));
              response.on("end", () => {
                held.push(() => {
                  outgoing.writeHead(response.statusCode, response.headers);
                  outgoing.end(Buffer.concat(chunks));
                });
                counts.committed++; report();
              });
              return;
            }
            counts.committed++; report();
            if (mode === "drop_after") { response.resume(); outgoing.destroy(); return; }
          }
          outgoing.writeHead(response.statusCode, response.headers);
          response.pipe(outgoing);
        });
        remote.on("error", () => { if (!outgoing.headersSent) outgoing.writeHead(502); outgoing.end(); });
        if (body) remote.end(body); else incoming.pipe(remote);
      };
      if (mode === "hold_before") held.push(forward); else forward();
    } catch { outgoing.writeHead(503); outgoing.end(); }
  });
  await new Promise((resolve, reject) => { server.once("error", reject); server.listen(port, "127.0.0.1", resolve); });
  report();
  return { close: async () => { clearInterval(timer); server.closeAllConnections(); await new Promise(resolve => server.close(resolve)); } };
}

export class ControlProxy {
  counts = { matched: 0, committed: 0 };
  async start(fixture) {
    assert(process.getuid() > 0, "run the fixture as its non-root delegated service user");
    this.config = join(fixture.directory, "control-fault.json");
    await atomic(this.config, { id: "none" });
    const upstream = await freePort();
    const port = await freePort();
    fixture.config.lifecycle.internal_listen = `127.0.0.1:${upstream}`;
    this.child = spawn("sudo", ["-n", process.execPath, resolve("scripts/forgejo-lifecycle/control-proxy.mjs"),
      "--serve", String(port), String(upstream), this.config], { stdio: ["ignore", "pipe", "ignore"] });
    let text = "";
    this.child.stdout.on("data", bytes => {
      text += bytes.toString("utf8");
      while (text.includes("\n")) {
        const end = text.indexOf("\n");
        this.counts = JSON.parse(text.slice(0, end));
        text = text.slice(end + 1);
      }
    });
    this.child.on("error", () => { this.failed = true; });
    await until("root loopback fault proxy ready", () => {
      assert(!this.failed && this.child.exitCode === null, "fault proxy exited");
      return this.counts.pid;
    }, 10000);
    this.rule = ["OUTPUT", "-d", "127.0.0.1/32", "-p", "tcp", "--dport", String(upstream),
      "-m", "owner", "--uid-owner", String(process.getuid()), "-m", "comment", "--comment", fixture.prefix,
      "-j", "REDIRECT", "--to-ports", String(port)];
    await command("sudo", ["-n", "iptables", "-w", "5", "-t", "nat", "-A", ...this.rule]);
    this.installed = true;
    return this;
  }
  async arm(kind, mode, limit = 1, intent) {
    const id = randomUUID();
    await atomic(this.config, { id, kind, mode, limit, intent });
    await until("fault arm acknowledged", () => this.counts.id === id, 5000);
  }
  async release() { await this.arm("none", "none", 0); }
  async close() {
    if (this.installed) await command("sudo", ["-n", "iptables", "-w", "5", "-t", "nat", "-D", ...this.rule]);
    if (this.counts.pid) await command("sudo", ["-n", "kill", "-TERM", String(this.counts.pid)]);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  assert.equal(process.argv[2], "--serve");
  assert.equal(process.getuid(), 0);
  const port = Number(process.argv[3]);
  const upstream = Number(process.argv[4]);
  assert(Number.isInteger(port) && port > 1024 && port < 65536 && Number.isInteger(upstream) && upstream > 1024 && upstream < 65536);
  const server = await serveProxy(port, upstream, process.argv[5], value => process.stdout.write(`${JSON.stringify(value)}\n`));
  process.on("SIGTERM", async () => { await server.close(); process.exit(0); });
}
