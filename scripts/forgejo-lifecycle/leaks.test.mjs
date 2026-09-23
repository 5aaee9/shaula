import { test } from "node:test";
import assert from "node:assert/strict";
import { protectedState } from "./leaks.mjs";

const fixture = { platform: "kubernetes", prefix: "namespace" };
const observed = { id: "generation", secretUid: "original-uid" };
const secrets = ["management-secret", "single-runner-secret"];
function plan() {
  return { resource_changes: [{ type: "kubernetes_secret_v1", name: "bootstrap", change: { before: {
    metadata: [{ name: "generation", namespace: "namespace", uid: "original-uid" }], data: { token: secrets[1] },
  } } }] };
}
test("only the exact bootstrap Secret data may retain a Runner token in protected Kubernetes evidence", () => {
  assert.equal(protectedState(JSON.stringify(plan()), fixture, observed, secrets), 1);
  assert.throws(() => protectedState(JSON.stringify(plan()), { ...fixture, platform: "docker" }, observed, secrets));
  for (const modify of [
    p => { p.variables = { token: secrets[1] }; },
    p => { p.resource_changes[0].change.before.metadata[0].annotation = secrets[1]; },
    p => { p.resource_changes[0].change.before.metadata[0].uid = "replacement"; },
    p => { p.resource_changes[0].change.before.data.token = secrets[0]; },
    p => { p.resource_changes[0].change.before.data.another = secrets[1]; },
    p => { p.resource_changes[0].name = "other"; },
    p => { p.env = Buffer.from(secrets[1]).toString("base64"); },
  ]) {
    const value = plan();
    modify(value);
    assert.throws(() => protectedState(JSON.stringify(value), fixture, observed, secrets));
  }
});
