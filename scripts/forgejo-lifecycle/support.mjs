// Disposable acceptance fixtures only. Never echo CLI output or response bodies.
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { createServer } from "node:net";
import { setTimeout as delay } from "node:timers/promises";

const execute = promisify(execFile);
export const serverImage = "codeberg.org/forgejo/forgejo:16.0.4";
export const runnerImage = "code.forgejo.org/forgejo/runner:13.1.0@sha256:c4af85fd9f0dd03788676a534781a87c71aa2c6a37737143e017eb94d4312952";
export async function command(binary, args, options = {}) {
  try {
    return (await execute(binary, args, { timeout: 180_000, maxBuffer: 8 * 1024 * 1024, ...options })).stdout.trim();
  } catch {
    throw new Error(`fixture command failed: ${binary.split("/").at(-1)} ${args[0]}`);
  }
}
export async function until(label, predicate, timeout = 120_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const value = await predicate();
    if (value) return value;
    await delay(250);
  }
  throw new Error(`timed out: ${label}`);
}
export async function freePort() {
  const server = createServer();
  await new Promise((resolve, reject) => { server.once("error", reject); server.listen(0, "127.0.0.1", resolve); });
  const port = server.address().port;
  await new Promise(resolve => server.close(resolve));
  return port;
}
export async function json(url, token, method = "GET", body, expected = 200, headers = {}) {
  const response = await fetch(url, { method, headers: { authorization: `Bearer ${token}`, "content-type": "application/json", ...headers },
    body: body === undefined ? undefined : JSON.stringify(body), redirect: "error", signal: AbortSignal.timeout(15_000) });
  assert.equal(response.status, expected, `${method} ${new URL(url).pathname}: HTTP status`);
  return expected === 204 || expected === 404 ? null : response.json();
}
