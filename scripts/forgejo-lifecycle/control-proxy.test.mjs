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
  const server = createServer(async (req, res) => {
    let body = ""; for await (const chunk of req) body += chunk;
    requests.push(body); res.writeHead(200); res.end('{"kind":"ack"}');
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
  } finally {
    await proxy.close(); server.closeAllConnections(); await new Promise(resolve => server.close(resolve));
    await rm(directory, { recursive: true });
  }
});
