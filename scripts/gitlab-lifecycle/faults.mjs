// Test-only GitLab API transport faults between the daemon (and Generation
// runners) and the disposable GitLab. The production binary is unchanged:
// the Fleet's instance_url names this proxy. Response bodies are never read
// (registration responses carry a one-shot token); from requests only the
// non-secret registration fields are kept, and headers never.
import { createServer, request } from "node:http";

const RUNNER = /^\/api\/v4\/runners\/(\d+)$/;
const KEPT = ["runner_type", "group_id", "project_id", "description", "maintenance_note", "tag_list", "run_untagged", "locked", "paused", "access_level"];

/** Request class; runner job polling and traces are always `other`. */
export function classify(method, path) {
  if (method === "POST" && path === "/api/v4/user/runners") return "register";
  if (method === "POST" && path === "/api/graphql") return "graphql";
  if (method === "PUT" && RUNNER.test(path)) return "pause";
  if (method === "DELETE" && RUNNER.test(path)) return "delete";
  if (method === "GET" && /^\/api\/v4\/(?:groups|projects)\/[^/]+\/runners$/.test(path)) return "inventory";
  return "other";
}

/** Registration request fields only; `role` comes from the ownership note. */
export function registrationFields(body) {
  const parsed = JSON.parse(body.toString("utf8"));
  const fields = Object.fromEntries(KEPT.filter(key => key in parsed).map(key => [key, parsed[key]]));
  fields.role = /;role=(anchor|generation);/.exec(fields.maintenance_note ?? "")?.[1];
  return fields;
}

/** Whether `rule` selects this request: kind, then optional role/prefix/runner. */
export function selects(rule, kind, path, fields) {
  if (!rule || rule.kind !== kind) return false;
  if (rule.role && fields?.role !== rule.role) return false;
  if (rule.prefix && !fields?.description?.startsWith(rule.prefix)) return false;
  if (rule.runner !== undefined && RUNNER.exec(path)?.[1] !== String(rule.runner)) return false;
  return true;
}

/**
 * Modes, all limited to `rule.limit` matches (default 1):
 * - reject: 503 before forwarding (GitLab never sees it)
 * - reset: socket destroyed before forwarding (response loss, never applied)
 * - drop_after: forwarded; a 2xx is discarded and the socket destroyed
 * - hold_after: forwarded; a 2xx is discarded and the socket parked until release()
 * - hold_before: parked unforwarded until release(), then forwarded normally
 */
export async function gitlabFaultProxy({ target, listen }) {
  const upstream = new URL(target);
  const gate = { rule: undefined, matched: 0, committed: 0, held: [], counts: {}, registrations: [] };
  const server = createServer(async (incoming, outgoing) => {
    try {
      const path = new URL(incoming.url, "http://fixture.invalid").pathname;
      const kind = classify(incoming.method, path);
      gate.counts[kind] = (gate.counts[kind] ?? 0) + 1;
      let body;
      let fields;
      if (kind === "register" || kind === "pause") {
        const chunks = [];
        let size = 0;
        for await (const chunk of incoming) {
          size += chunk.length;
          if (size > 65536) { outgoing.writeHead(413); outgoing.end(); return; }
          chunks.push(chunk);
        }
        body = Buffer.concat(chunks);
        if (kind === "register") {
          fields = registrationFields(body);
          gate.registrations.push(fields);
        }
      }
      const rule = gate.rule;
      const selected = selects(rule, kind, path, fields) && gate.matched < (rule.limit ?? 1);
      const mode = selected ? rule.mode : undefined;
      if (selected) gate.matched++;
      if (fields) fields.fault = mode;
      if (mode === "reject") { outgoing.writeHead(503, { "content-type": "application/json" }); outgoing.end('{"message":"disposable fault"}'); return; }
      if (mode === "reset") { outgoing.destroy(); return; }
      const forward = () => {
        const headers = { ...incoming.headers, host: upstream.host };
        const remote = request({ host: upstream.hostname, port: upstream.port, path: incoming.url,
          method: incoming.method, headers }, response => {
          if (mode && response.statusCode >= 200 && response.statusCode < 300 && ["drop_after", "hold_after"].includes(mode)) {
            // The server committed. Discard the body unread; never log it.
            response.resume();
            response.on("end", () => {
              gate.committed++;
              if (mode === "drop_after") outgoing.destroy();
              else gate.held.push(() => outgoing.destroy());
            });
            return;
          }
          outgoing.writeHead(response.statusCode, response.headers);
          response.pipe(outgoing);
        });
        remote.on("error", () => { if (!outgoing.headersSent) outgoing.writeHead(502); outgoing.end(); });
        if (body) remote.end(body); else incoming.pipe(remote);
      };
      if (mode === "hold_before") gate.held.push(forward); else forward();
    } catch { if (!outgoing.headersSent) outgoing.writeHead(503); outgoing.end(); }
  });
  await new Promise((resolve, reject) => { server.once("error", reject); server.listen(listen, resolve); });
  return {
    gate,
    address: server.address(),
    arm(rule) { gate.rule = rule; gate.matched = 0; gate.committed = 0; },
    /** Disarms the rule and lets every parked request/response proceed. */
    release() { gate.rule = undefined; for (const resume of gate.held.splice(0)) resume(); },
    /** Registration requests (fields only) under a description prefix. */
    posts(prefix) { return gate.registrations.filter(r => r.description?.startsWith(prefix)); },
    close() { gate.rule = undefined; for (const resume of gate.held.splice(0)) resume(); return new Promise(resolve => { server.closeAllConnections(); server.close(resolve); }); },
  };
}
