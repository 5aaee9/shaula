import assert from "node:assert/strict";
import { symlink } from "node:fs/promises";
import { join } from "node:path";
import { until } from "./support.mjs";
import { diagnostic, ledger, hasReason } from "./diagnostics-evidence.mjs";
import { captureCredentials } from "./leaks.mjs";

async function generation(f, key, predicate) {
  return until(`${key}: generation checkpoint`, async () => {
    const rows = (await f.api(`/generations?fleet_key=${key}`)).items;
    return rows.find(predicate);
  });
}

export async function noCreate(f, evidence) {
  const key = `${f.prefix}-nocreate`;
  const gate = f.dockerProxy.gate;
  const before = { creates: gate.containerCreates, deletes: gate.containerDeletes };
  gate.failImageRead = true;
  await f.createFleet(key, 1);
  await until("real image read fails during plan", () => gate.imageFailures > 0);
  // Remove desired capacity before cleanup permits a new admission.
  await f.capZero(key);
  const g = await generation(f, key, g => g.state === "Destroyed");
  // The terminal ledger dominates runtime reasons. Its completed reason must
  // preserve the controller's never_started provenance, not claim provider cleanup.
  await until("never-started completion provenance", async () => (await f.api(`/generations/${g.id}/diagnostics`)).questions
    .some(q => q.reasons.some(r => r.parameters?.completionSource === "never_started")));
  const receipt = await evidence.capture("no-create", key, "generation", g.id, "cleanup.completed");
  assert.equal(receipt.capacity.occupancy, 0);
  assert.equal(receipt.provider.containers.length, 0);
  assert.equal(receipt.provider.runners.length, 0);
  assert.equal(gate.containerCreates, before.creates, "no resource create request");
  assert.equal(gate.containerDeletes, before.deletes, "no resource destroy request");
  assert(receipt.ledger.invocations.some(i => i.operation === "Create" && i.outcome === "failed"
    && i.commands.some(c => c.phase === "plan" && c.exitCode !== 0)), "real failed plan invocation");
  assert(receipt.ledger.invocations.every(i => i.commands.every(c => c.phase !== "apply")), "apply was never started");
  assert(receipt.ledger.operations.every(o => o.kind !== "Destroy"), "never-started cleanup needs no Terraform Destroy");
  assert(receipt.http.questions.some(q => q.reasons.some(r => r.parameters?.completionSource === "never_started")));
  assert(receipt.uiText.includes("Create apply never started"), "UI distinguishes no-create from provider cleanup");
  gate.failImageRead = false;
}

export async function waitingOnline(f, evidence) {
  const key = `${f.prefix}-waiting`;
  f.registrationProxy.gate.blockDeclare = true;
  await f.createFleet(key, 1);
  const g = await generation(f, key, g => g.state === "WaitingOnline");
  await until("real runner Declare fails", () => f.registrationProxy.gate.declareFailures > 0);
  const ids = (await f.cli("ps", "-a", "--no-trunc", "-q", "--filter", `label=shaula.fleet=${key}`)).split("\n").filter(Boolean);
  assert.equal(ids.length, 1);
  // The one-job process may exit after its initial Declare failure. Read its
  // token through the still-owned volume without starting a replacement runner.
  const tokenFile = join(f.directory, "waiting-token");
  await f.cli("cp", `${ids[0]}:/data/.forgejo-token`, tokenFile);
  const { readFile } = await import("node:fs/promises");
  evidence.secrets.push((await readFile(tokenFile, "utf8")).trim());
  await diagnostic(f, "generation", g.id, "lifecycle.awaiting_online");
  const receipt = await evidence.capture("waiting-online", key, "generation", g.id, "lifecycle.awaiting_online");
  assert.equal(receipt.ledger.generations.find(row => row.id === g.id).state, "WaitingOnline");
  assert.equal(receipt.provider.containers.length, 1);
  assert.equal(receipt.capacity.occupancy, 1);
  assert.equal(receipt.provider.runners.length, 1);
  assert(!["idle", "active"].includes(receipt.provider.runners[0].status));
  f.registrationProxy.gate.blockDeclare = false;
  // Restart the same faulted external process, preserving the original
  // registration and container. This is fault recovery, not a Shaula Create.
  await f.cli("restart", ids[0]);
  const idle = await f.observe(key, "idle");
  await until("controller observes online recovery", () => ledger(f, key).generations[0].state === "Idle");
  const recovered = await f.api(`/generations/${g.id}/diagnostics`);
  await f.capZero(key);
  await f.queue(key, 8);
  await f.reclaimed(key, idle);
  await evidence.capture("waiting-online-recovered", key, "generation", g.id, "cleanup.completed");
  assert.equal(recovered.subject.id, g.id);
}

export async function destroyFailure(f, evidence) {
  const key = `${f.prefix}-destroy`;
  await f.queue(key, 180);
  await f.createFleet(key);
  const busy = await f.observe(key, "active");
  evidence.secrets.push(...await captureCredentials(f, busy));
  const g = await generation(f, key, () => true);
  f.dockerProxy.gate.block = true;
  await f.capZero(key);
  await f.restart(15);
  await until("real Docker DELETE fails", () => f.dockerProxy.gate.failures > 0);
  await until("failed destroy is durable", () => {
    const facts = ledger(f, key);
    return facts.generations[0].state === "DestroyPending" && facts.invocations.some(i => i.operation === "Destroy"
      && i.outcome === "failed" && i.commands.some(c => c.phase === "apply" && c.exitCode !== 0));
  });
  // A failed DELETE after apply started does not prove that apply had no effects.
  // The production ledger deliberately retains ApplyStarting/uncertainty.
  await diagnostic(f, "generation", g.id, "lifecycle.apply_outcome_unknown");
  const receipt = await evidence.capture("destroy-failure", key, "generation", g.id, "lifecycle.apply_outcome_unknown");
  assert.equal(receipt.capacity.occupancy, 1);
  assert.equal(receipt.provider.containers.length, 1);
  assert(receipt.ledger.generations[0].expiry_requested_at !== null);
  assert(receipt.http.questions.some(q => q.cleanupMode === "hard_lifetime"));
  // Restore the failed resource transport across a daemon restart. Forgejo
  // may remove the ephemeral registration itself when the one-job exits;
  // require actual absence on both sides, not a particular DELETE request.
  await f.restart(15);
  f.dockerProxy.gate.block = false;
  await f.reclaimed(key, busy);
  await evidence.capture("destroy-recovered", key, "generation", g.id, "cleanup.completed");
  await f.restart(300);
}

export async function rolloutLag(f, evidence) {
  const key = `${f.prefix}-rollout`;
  await f.createFleet(key, 1);
  const idle = await f.observe(key, "idle");
  evidence.secrets.push(...await captureCredentials(f, idle));
  await f.capZero(key);
  const alias = join(f.directory, "docker-revision-2.sock");
  await symlink(f.proxySocket, alias);
  const response = await fetch(`${f.url}/api/v1/template-profiles/forgejo`, { headers: { authorization: `Bearer ${f.oidc.token}` } });
  assert.equal(response.status, 200);
  await f.api("/template-profiles/forgejo", "PUT", {
    artifact_digest: f.digest, engine_ref: "terraform", bindings: { ...f.bindings(), docker_host: `unix://${alias}` }, fleet_input_policy: {},
  }, 202, { "if-match": response.headers.get("shaula-resource-version") || response.headers.get("etag") });
  await until("real published revision Active", async () => (await f.api("/template-profiles/forgejo")).activeRevision === 2);
  await diagnostic(f, "fleet", key, "rollout.waiting_zero_occupancy");
  const receipt = await evidence.capture("rollout-lag", key, "fleet", key, "rollout.waiting_zero_occupancy");
  const rollout = receipt.http.questions.find(q => q.question === "rollout").rollout;
  assert.equal(rollout.previousPin.revision, "1");
  assert.equal(rollout.candidatePin.revision, "2");
  assert.equal(receipt.capacity.occupancy, 1);
  assert.equal(receipt.ledger.revisions.at(-1).template_revision, 1);
  await f.queue(key, 8);
  await f.reclaimed(key, idle);
  await until("follow commits only after zero occupancy", () => ledger(f, key).revisions.at(-1).template_revision === 2);
  await until("old rollout barrier clears", async () => !hasReason(await f.api(`/fleets/${key}/diagnostics`), "rollout.waiting_zero_occupancy"));
  await evidence.capture("rollout-recovered", key, "fleet", key, "capacity.target_satisfied");
}
