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

test("demand failure and a lost registration response never intercept task acquisition", async () => {
  let posts = 0;
  const upstream = createServer((req,res) => {
    req.resume();
    if (req.method === "POST" && req.url === "/api/v1/admin/actions/runners") {
      posts++; res.writeHead(201, { "content-type": "application/json" }); res.end('{"id":1,"token":"fixture-canary"}');
    } else { res.writeHead(204); res.end(); }
  });
  await new Promise(resolve => upstream.listen(0,"127.0.0.1",resolve));
  const port = await freePort();
  const proxy = await faultProxy({target:`http://127.0.0.1:${upstream.address().port}`,listen:{host:"127.0.0.1",port}});
  const url = `http://127.0.0.1:${port}`;
  try {
    proxy.gate.failJobs = true;
    assert.equal((await fetch(`${url}/api/v1/admin/actions/runners/jobs?labels=linux`)).status,503);
    assert.equal((await fetch(`${url}/twirp/runner.v1.RunnerService/FetchTask`,{method:"POST"})).status,204);
    proxy.gate.dropRegistrationResponses = true;
    await assert.rejects(fetch(`${url}/api/v1/admin/actions/runners`,{method:"POST"}));
    assert.equal(posts,1);
    assert.equal(proxy.gate.registrationPosts,1);
    assert.equal(proxy.gate.droppedRegistrations,1);
    assert.equal(proxy.gate.jobFailures,1);
  } finally { await proxy.close(); upstream.closeAllConnections(); await new Promise(resolve=>upstream.close(resolve)); }
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

test("DX-30 plan fault prevents only image reads and counts actual resource requests", async () => {
  const dir = await mkdtemp(join(tmpdir(), "shaula-dx30-fault-"));
  const socketPath = join(dir, "engine.sock");
  const upstream = createServer((req, res) => { req.resume(); res.writeHead(204); res.end(); });
  await new Promise(resolve => upstream.listen(socketPath, resolve));
  const endpoint = join(dir, "proxy.sock");
  const proxy = await faultProxy({ socketPath, listen: endpoint });
  try {
    proxy.gate.failImageRead = true;
    assert.equal(await call(endpoint, "/v1.41/images/runner/json", "GET"), 503);
    assert.equal(await call(endpoint, "/v1.41/containers/abcd/json", "GET"), 204);
    assert.equal(await call(endpoint, "/v1.41/containers/create?name=test", "POST"), 204);
    assert.equal(await call(endpoint, "/v1.41/containers/abcd?v=1", "DELETE"), 204);
    assert.equal(proxy.gate.imageFailures, 1);
    assert.equal(proxy.gate.containerCreates, 1);
    assert.equal(proxy.gate.containerDeletes, 1);
    proxy.gate.failImageRead = false;
    assert.equal(await call(endpoint, "/v1.41/images/runner/json", "GET"), 204);
  } finally { await proxy.close(); await new Promise(resolve => upstream.close(resolve)); await rm(dir, { recursive: true }); }
});

test("DX-30 readiness fault blocks Declare while acquisition and inventory pass unchanged", async () => {
  const upstream = createServer((req, res) => { req.resume(); res.writeHead(204); res.end(); });
  await new Promise(resolve => upstream.listen(0, "127.0.0.1", resolve));
  const port = await freePort();
  const proxy = await faultProxy({ target: `http://127.0.0.1:${upstream.address().port}`, listen: { host: "127.0.0.1", port } });
  const url = `http://127.0.0.1:${port}`;
  try {
    proxy.gate.blockDeclare = true;
    assert.equal((await fetch(`${url}/api/actions/runner.v1.RunnerService/Declare`, { method: "POST" })).status, 503);
    assert.equal((await fetch(`${url}/api/actions/runner.v1.RunnerService/FetchTask`, { method: "POST" })).status, 204);
    assert.equal((await fetch(`${url}/api/v1/admin/actions/runners`)).status, 204);
    assert.equal(proxy.gate.declareFailures, 1);
    proxy.gate.blockDeclare = false;
    assert.equal((await fetch(`${url}/api/actions/runner.v1.RunnerService/Declare`, { method: "POST" })).status, 204);
  } finally { await proxy.close(); upstream.closeAllConnections(); await new Promise(resolve => upstream.close(resolve)); }
});
