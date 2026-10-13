import assert from "node:assert/strict";
import { test } from "node:test";
import { createServer, request } from "node:http";
import { gitlabFaultProxy, classify } from "./faults.mjs";
import { until } from "../forgejo-lifecycle/support.mjs";

const TOKEN = "glrt-fixture-only-token";

async function upstream() {
  const seen = [];
  const server = createServer(async (req, res) => {
    const chunks = [];
    for await (const chunk of req) chunks.push(chunk);
    seen.push({ method: req.method, url: req.url, host: req.headers.host });
    if (req.url === "/api/v4/user/runners") { res.writeHead(201); res.end(JSON.stringify({ id: seen.length, token: TOKEN })); return; }
    res.writeHead(200); res.end("{}");
  });
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  return { seen, url: `http://127.0.0.1:${server.address().port}`, close: () => new Promise(resolve => server.close(resolve)) };
}

function call(port, path, method, body) {
  return new Promise(resolve => {
    const req = request({ host: "127.0.0.1", port, path, method, headers: { "content-type": "application/json" } }, res => {
      const chunks = [];
      res.on("data", chunk => chunks.push(chunk));
      res.on("end", () => resolve({ status: res.statusCode, body: Buffer.concat(chunks).toString() }));
    });
    req.on("error", () => resolve({ status: "lost" }));
    req.end(body === undefined ? undefined : JSON.stringify(body));
  });
}

const generation = description => ({ runner_type: "group_type", group_id: 7, description,
  maintenance_note: `shaula:fleet=f;role=generation;name=${description}`, tag_list: ["f"], run_untagged: false });

test("runner job traffic is never classified as a management request", () => {
  assert.equal(classify("POST", "/api/v4/jobs/request"), "other");
  assert.equal(classify("POST", "/api/v4/runners/verify"), "other");
  assert.equal(classify("PATCH", "/api/v4/jobs/9/trace"), "other");
  assert.equal(classify("PUT", "/api/v4/runners/12"), "pause");
  assert.equal(classify("GET", "/api/v4/groups/3/runners"), "inventory");
});

test("registration response loss: reset never reaches GitLab, drop_after commits once", async () => {
  const server = await upstream();
  const proxy = await gitlabFaultProxy({ target: server.url, listen: { host: "127.0.0.1", port: 0 } });
  try {
    proxy.arm({ kind: "register", mode: "reset", role: "generation", prefix: "f-" });
    assert.equal((await call(proxy.address.port, "/api/v4/user/runners", "POST", generation("f-a"))).status, "lost");
    assert.equal(server.seen.length, 0, "reset must not forward");
    proxy.arm({ kind: "register", mode: "drop_after", role: "generation", prefix: "f-" });
    assert.equal((await call(proxy.address.port, "/api/v4/user/runners", "POST", generation("f-b"))).status, "lost");
    assert.equal(server.seen.length, 1);
    assert.equal(proxy.gate.committed, 1);
    // Limit 1: the next registration passes through untouched.
    assert.equal((await call(proxy.address.port, "/api/v4/user/runners", "POST", generation("f-c"))).status, 201);
    assert.deepEqual(proxy.posts("f-").map(r => [r.description, r.role, r.fault]),
      [["f-a", "generation", "reset"], ["f-b", "generation", "drop_after"], ["f-c", "generation", undefined]]);
    assert(!JSON.stringify(proxy.gate).includes(TOKEN), "proxy state never keeps a response token");
    assert.equal(server.seen[0].host, new URL(server.url).host, "Host is rewritten to the upstream");
  } finally { await proxy.close(); await server.close(); }
});

test("hold_after parks a committed response; hold_before parks the request itself", async () => {
  const server = await upstream();
  const proxy = await gitlabFaultProxy({ target: server.url, listen: { host: "127.0.0.1", port: 0 } });
  try {
    proxy.arm({ kind: "register", mode: "hold_after", role: "anchor" });
    const anchor = { ...generation("f-anchor-r1"), maintenance_note: "shaula:fleet=f;role=anchor;name=f-anchor-r1" };
    const lost = call(proxy.address.port, "/api/v4/user/runners", "POST", anchor);
    await until("committed and held", () => proxy.gate.held.length === 1, 5000);
    assert.equal(server.seen.length, 1);
    proxy.release();
    assert.equal((await lost).status, "lost");
    proxy.arm({ kind: "pause", mode: "hold_before", runner: 12 });
    assert.equal((await call(proxy.address.port, "/api/v4/runners/13", "PUT", { paused: true })).status, 200, "other runners pass");
    const paused = call(proxy.address.port, "/api/v4/runners/12", "PUT", { paused: true });
    await until("pause held", () => proxy.gate.held.length === 1, 5000);
    assert.equal(server.seen.filter(r => r.url === "/api/v4/runners/12").length, 0);
    proxy.release();
    assert.equal((await paused).status, 200);
    assert.equal(server.seen.filter(r => r.url === "/api/v4/runners/12").length, 1);
  } finally { await proxy.close(); await server.close(); }
});

test("reject answers 503 for the demand query without forwarding", async () => {
  const server = await upstream();
  const proxy = await gitlabFaultProxy({ target: server.url, listen: { host: "127.0.0.1", port: 0 } });
  try {
    proxy.arm({ kind: "graphql", mode: "reject", limit: Infinity });
    for (let i = 0; i < 3; i++) assert.equal((await call(proxy.address.port, "/api/graphql", "POST", { query: "{}" })).status, 503);
    assert.equal((await call(proxy.address.port, "/api/v4/jobs/request", "POST", {})).status, 200);
    assert.equal(proxy.gate.matched, 3);
    assert.deepEqual(server.seen.map(r => r.url), ["/api/v4/jobs/request"]);
  } finally { await proxy.close(); await server.close(); }
});
