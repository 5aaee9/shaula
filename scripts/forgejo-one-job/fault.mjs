// Test-only HTTP fault injector, NOT a production acquisition proxy or drain proof.
import http from "node:http";

export async function faultEndpoint(upstreamUrl, fault) {
  const upstreamBase = new URL(upstreamUrl);
  if (upstreamBase.protocol !== "http:" || upstreamBase.hostname !== "127.0.0.1") {
    throw new Error("fault injector only supports the disposable loopback server");
  }
  if (!["unavailable", "lose-response"].includes(fault)) throw new Error("unknown test fault");
  const evidence = { fetches: 0, forwardedFetches: 0, droppedResponses: 0, upstreamStatus: null };
  const requests = new Set();
  const server = http.createServer((request, response) => {
    const fetching = request.url.split("?")[0].endsWith("/FetchTask");
    if (fetching) evidence.fetches++;
    if (fetching && fault === "unavailable") {
      request.resume();
      response.writeHead(503, { "content-type": "application/json" });
      response.end(
        JSON.stringify({ code: "unavailable", message: "test fault before assignment" }),
      );
      return;
    }
    if (fetching) evidence.forwardedFetches++;
    const upstream = http.request(
      {
        hostname: upstreamBase.hostname,
        port: upstreamBase.port,
        path: request.url,
        method: request.method,
        headers: { ...request.headers, host: upstreamBase.host },
        timeout: 15_000,
      },
      (received) => {
        if (fetching && fault === "lose-response" && evidence.droppedResponses === 0) {
          evidence.upstreamStatus = received.statusCode;
          received.resume();
          // Wait until Forgejo has finished assigning/responding, then hide the
          // entire response. No payloads, credentials or task contents are logged.
          received.on("end", () => {
            evidence.droppedResponses++;
            response.destroy();
          });
        } else {
          response.writeHead(received.statusCode, received.headers);
          received.pipe(response);
        }
        received.on("error", () => response.destroy());
      },
    );
    requests.add(upstream);
    upstream.on("close", () => requests.delete(upstream));
    upstream.on("timeout", () => upstream.destroy());
    upstream.on("error", () => response.destroy());
    request.on("error", () => upstream.destroy());
    request.pipe(upstream);
  });
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  return {
    url: `http://127.0.0.1:${server.address().port}`,
    evidence,
    async close() {
      const closed = new Promise((resolve) => server.close(resolve));
      server.closeAllConnections();
      for (const request of requests) request.destroy();
      await closed;
    },
  };
}
