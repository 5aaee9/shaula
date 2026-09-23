// Test-only transport faults. No task acquisition/assignment interception.
import { createServer, request } from "node:http";

export async function faultProxy({ socketPath, target, listen }) {
  const gate = { block: false, failures: 0, failJobs: false, jobFailures: 0,
    dropRegistrationResponses: false, registrationPosts: 0, droppedRegistrations: 0 };
  const server = createServer((incoming, outgoing) => {
    const path = new URL(incoming.url, "http://fixture.invalid").pathname;
    const register = target && incoming.method === "POST" && path === "/api/v1/admin/actions/runners";
    if (register) gate.registrationPosts++;
    if (gate.failJobs && target && incoming.method === "GET" && path === "/api/v1/admin/actions/runners/jobs") {
      gate.jobFailures++;
      outgoing.writeHead(503, { "content-type": "application/json" });
      outgoing.end('{"message":"disposable demand fault"}');
      return;
    }
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
      if (register && gate.dropRegistrationResponses && response.statusCode === 201) {
        // Let the real server commit, then discard the one-shot response. Never log its body.
        response.on("end", () => { gate.droppedRegistrations++; outgoing.destroy(); });
        response.resume();
        return;
      }
      outgoing.writeHead(response.statusCode, response.headers);
      response.pipe(outgoing);
    });
    remote.on("error", () => { if (!outgoing.headersSent) outgoing.writeHead(502); outgoing.end(); });
    incoming.pipe(remote);
  });
  await new Promise((resolve, reject) => { server.once("error", reject); server.listen(listen, resolve); });
  return { gate, close: () => new Promise(resolve => { server.closeAllConnections(); server.close(resolve); }) };
}
