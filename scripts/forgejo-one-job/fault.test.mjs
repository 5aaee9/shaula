import assert from "node:assert/strict";
import http from "node:http";
import test from "node:test";
import { faultEndpoint } from "./fault.mjs";

async function upstream(t) {
  let calls = 0;
  const server = http.createServer((request, response) => {
    calls++;
    request.resume();
    request.on("end", () => {
      response.writeHead(200, { "content-type": "application/proto" });
      response.end("assigned-task");
    });
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  t.after(async () => {
    const closed = new Promise((resolve) => server.close(resolve));
    server.closeAllConnections();
    await closed;
  });
  return {
    url: `http://127.0.0.1:${server.address().port}`,
    calls: () => calls,
  };
}

function fetchTask(endpoint) {
  return fetch(`${endpoint.url}/api/actions/runner.v1.RunnerService/FetchTask`, {
    method: "POST",
    body: "request",
    signal: AbortSignal.timeout(3_000),
  });
}

test("unavailable fault never forwards task acquisition", async (t) => {
  const source = await upstream(t);
  const endpoint = await faultEndpoint(source.url, "unavailable");
  t.after(() => endpoint.close());
  assert.equal((await fetchTask(endpoint)).status, 503);
  assert.equal(source.calls(), 0);
  assert.deepEqual(endpoint.evidence, {
    fetches: 1,
    forwardedFetches: 0,
    droppedResponses: 0,
    upstreamStatus: null,
  });
  const other = await fetch(`${endpoint.url}/api/actions/runner.v1.RunnerService/Declare`);
  assert.equal(await other.text(), "assigned-task");
  assert.equal(source.calls(), 1);
});

test("response loss occurs after upstream success and only once", async (t) => {
  const source = await upstream(t);
  const endpoint = await faultEndpoint(source.url, "lose-response");
  t.after(() => endpoint.close());
  await assert.rejects(fetchTask(endpoint));
  assert.equal(source.calls(), 1);
  assert.deepEqual(endpoint.evidence, {
    fetches: 1,
    forwardedFetches: 1,
    droppedResponses: 1,
    upstreamStatus: 200,
  });
  const retried = await fetchTask(endpoint);
  assert.equal(await retried.text(), "assigned-task");
  assert.equal(source.calls(), 2);
  assert.equal(endpoint.evidence.droppedResponses, 1);
});

test("fault injector rejects non-local upstreams and unknown modes", async () => {
  await assert.rejects(faultEndpoint("https://forgejo.example", "lose-response"));
  await assert.rejects(faultEndpoint("http://127.0.0.1:12345", "other"));
});
