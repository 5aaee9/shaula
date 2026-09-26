import assert from "node:assert/strict";
import { test } from "node:test";
import { createServer } from "node:http";
import { mkdtemp, writeFile, rename, rm } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { serveProxy } from "./control-proxy.mjs";
import { freePort, until } from "./support.mjs";

test("control faults preserve committed requests and hold uncommitted requests separately", async () => {
  const directory = await mkdtemp(join(tmpdir(), "shaula-control-proxy-"));
  const path = join(directory, "gate.json");
  const configure = async value => {
    await writeFile(`${path}.next`, JSON.stringify(value)); await rename(`${path}.next`, path);
  };
  await configure({ id: "drop", kind: "spawned", mode: "drop_after", limit: 2 });
  const requests = [];
  let releaseUpstream;
  let delayed = false;
  const server = createServer(async (req, res) => {
    let body = ""; for await (const chunk of req) body += chunk;
    requests.push(body);
    if (delayed) await new Promise(resolve => { releaseUpstream = resolve; });
    res.writeHead(200); res.end('{"kind":"ack"}');
  });
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  const port = await freePort();
  let counts;
  const proxy = await serveProxy(port, server.address().port, path, value => { counts = value; });
  const body = JSON.stringify({ request_id: "one", message: { kind: "spawned", payload: { intent: "create" } } });
  const send = () => fetch(`http://127.0.0.1:${port}/internal/v1/generations/test/control`, { method: "POST", body });
  try {
    await assert.rejects(send()); await assert.rejects(send());
    assert.equal((await send()).status, 200);
    assert.deepEqual(requests, [body, body, body]);
    assert.equal(counts.committed, 2);
    await configure({ id: "hold", kind: "spawned", mode: "hold_before", limit: 1 });
    await until("hold armed", () => counts.id === "hold");
    const pending = send();
    await until("request held", () => counts.matched === 1);
    assert.equal(requests.length, 3, "not forwarded before release");
    await configure({ id: "release" });
    assert.equal((await pending).status, 200);
    assert.equal(requests.length, 4);
    await configure({ id: "outage", kind: "state_all", mode: "reject_before", limit: 10 });
    await until("state outage armed", () => counts.id === "outage");
    for (const method of ["GET", "POST", "LOCK", "UNLOCK"]) {
      const response = await fetch(`http://127.0.0.1:${port}/internal/v1/generations/test/state?ID=fixture`, { method });
      assert.equal(response.status, 503);
    }
    assert.equal(requests.length, 4, "all state methods fail before reaching SQLite");
    assert.equal((await send()).status, 200, "control remains separate from the state outage");
    await configure({ id: "load", kind: "logs", mode: "flood_logs", limit: 1 });
    await until("log pressure armed", () => counts.id === "load");
    const start = requests.length;
    const log = JSON.stringify({ request_id: "same-log-id", message: { kind: "append", payload: "nonsecret fixture" } });
    const response = await fetch(`http://127.0.0.1:${port}/internal/v1/generations/test/logs`, { method: "POST", body: log });
    assert.equal(response.status, 200);
    await until("bounded original plus duplicate deliveries", () => requests.length === start + 17);
    assert(requests.slice(start).every(value => value === log), "load must reuse the exact request without fabricating log entries");
    assert.equal(counts.logDuplicates, 16);
    await configure({ id: "slow", kind: "spawned", mode: "hold_after", limit: 1 });
    await until("slow response fault armed", () => counts.id === "slow");
    delayed = true;
    const late = send();
    await until("upstream received slow request", () => releaseUpstream);
    await configure({ id: "next-scenario" });
    await until("old gate released before upstream reply", () => counts.id === "next-scenario");
    releaseUpstream();
    assert.equal((await late).status, 200, "released gate must not strand late upstream replies");
    assert.equal(counts.committed, 0, "old responses cannot satisfy a later scenario's commit assertion");
  } finally {
    await proxy.close(); server.closeAllConnections(); await new Promise(resolve => server.close(resolve));
    await rm(directory, { recursive: true });
  }
});
