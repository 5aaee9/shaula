import assert from "node:assert/strict";
import { test } from "node:test";
import { createServer, request } from "node:http";
import { mkdtemp, rm } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { faultProxy } from "./faults.mjs";
import { freePort } from "./support.mjs";

function call(socketPath, path, method) {
  return new Promise((resolve, reject) => {
    const req = request({ socketPath, path, method }, res => { res.resume(); res.on("end", () => resolve(res.statusCode)); });
    req.on("error", reject); req.end();
  });
}

test("Docker fault only blocks removal; recovery forwards the original request", async () => {
  const dir = await mkdtemp(join(tmpdir(), "shaula-fault-"));
  const socketPath = join(dir, "engine.sock");
  const upstream = createServer((req, res) => { req.resume(); res.writeHead(204); res.end(); });
  await new Promise(resolve => upstream.listen(socketPath, resolve));
  const proxy = await faultProxy({ socketPath, listen: join(dir, "proxy.sock") });
  try {
    proxy.gate.block = true;
    assert.equal(await call(join(dir, "proxy.sock"), "/v1.41/containers/abcd?v=1", "DELETE"), 503);
    assert.equal(await call(join(dir, "proxy.sock"), "/v1.41/containers/abcd/start", "POST"), 204);
    assert.equal(await call(join(dir, "proxy.sock"), "/v1.41/containers/abcd/json", "GET"), 204);
    assert.equal(proxy.gate.failures, 1);
    proxy.gate.block = false;
    assert.equal(await call(join(dir, "proxy.sock"), "/v1.41/containers/abcd?v=1", "DELETE"), 204);
  } finally {
    await proxy.close(); await new Promise(resolve => upstream.close(resolve)); await rm(dir, { recursive: true });
  }
});

test("Forgejo fault does not intercept task acquisition or reads", async () => {
  const upstream = createServer((req, res) => { req.resume(); res.writeHead(204); res.end(); });
  await new Promise(resolve => upstream.listen(0, "127.0.0.1", resolve));
  const port = await freePort();
  const proxy = await faultProxy({ target: `http://127.0.0.1:${upstream.address().port}`, listen: { host: "127.0.0.1", port } });
  try {
    proxy.gate.block = true;
    for (const [path, method, expected] of [
      ["/api/v1/admin/actions/runners/12", "DELETE", 503],
      ["/api/v1/admin/actions/runners", "GET", 204],
      ["/api/v1/admin/actions/runners", "POST", 204],
      ["/twirp/runner.v1.RunnerService/FetchTask", "POST", 204],
    ]) assert.equal((await fetch(`http://127.0.0.1:${port}${path}`, { method })).status, expected);
    assert.equal(proxy.gate.failures, 1);
    proxy.gate.block = false;
    assert.equal((await fetch(`http://127.0.0.1:${port}/api/v1/admin/actions/runners/12`, { method: "DELETE" })).status, 204);
  } finally { await proxy.close(); upstream.closeAllConnections(); await new Promise(resolve => upstream.close(resolve)); }
});
