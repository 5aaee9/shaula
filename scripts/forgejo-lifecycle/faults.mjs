// Test-only transport faults. No task acquisition/assignment interception.
import { createServer, request } from "node:http";

export async function faultProxy({ socketPath, target, listen }) {
  const gate = { block: false, failures: 0 };
  const server = createServer((incoming, outgoing) => {
    const dockerDelete = socketPath && /^\/(?:v[\d.]+\/)?containers\/[a-f0-9]+(?:\?|$)/.test(incoming.url);
    const registrationDelete = target && /^\/api\/v1\/admin\/actions\/runners\/\d+$/.test(incoming.url);
    if (gate.block && incoming.method === "DELETE" && (dockerDelete || registrationDelete)) {
      gate.failures += 1;
      outgoing.writeHead(503, { "content-type": "application/json" });
      outgoing.end('{"message":"disposable lifecycle fault"}');
      return;
    }
    const destination = target ? new URL(incoming.url, target) : undefined;
    const remote = request({ socketPath, hostname: destination?.hostname, port: destination?.port,
      path: incoming.url, method: incoming.method, headers: incoming.headers }, response => {
      outgoing.writeHead(response.statusCode, response.headers);
      response.pipe(outgoing);
    });
    remote.on("error", () => { if (!outgoing.headersSent) outgoing.writeHead(502); outgoing.end(); });
    incoming.pipe(remote);
  });
  await new Promise((resolve, reject) => { server.once("error", reject); server.listen(listen, resolve); });
  return { gate, close: () => new Promise(resolve => { server.closeAllConnections(); server.close(resolve); }) };
}
