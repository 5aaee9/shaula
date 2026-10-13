// G3 (spec 0043 §3, §3.1): permission revoked, anchor deleted, anchor
// unpaused and a failed demand poll each keep the old demand snapshot,
// mark it stale (Degraded) and authorize no ordinary scale-down. The
// trigger is a capacity change that a proven pass would act on at once: a
// new revision retires the old-revision Idle Generation and lowers the
// target to zero. Once demand is provable again, the same Generation is
// retired, and every drifted anchor is replaced and deleted.
import assert from "node:assert/strict";
import { until } from "../forgejo-lifecycle/support.mjs";
import { anchors, containers, demandRow, diagnosticCodes, fleetGone, generations, named, pipeline,
  reclaimed, retired, runningGeneration, setCapacity, status, steady } from "./scenario-support.mjs";

/** The group access token is a bot member; its role is its permission. The
 * members API refuses to edit token bots (403), so change the membership
 * row itself. */
async function setTokenRole(fixture, level) {
  const tokens = await fixture.gitlab(`/groups/${fixture.group.id}/access_tokens`);
  const bot = tokens.find(t => t.name === "shaula" && t.active);
  assert(bot?.user_id, "operator token bot user");
  await fixture.cli("exec", fixture.server, "gitlab-rails", "runner",
    `GroupMember.find_by!(source_id: ${Number(fixture.group.id)}, user_id: ${Number(bot.user_id)}).update!(access_level: ${Number(level)})`);
}

async function phases(fixture, keys) {
  return Promise.all(keys.map(async key => (await status(fixture, key)).phase));
}

export async function demandSuite(fixture, checks, progress) {
  const { proxy } = fixture;
  // A Fleet that can never serve its pending job keeps a positive snapshot.
  const hold = `${fixture.prefix}-hold`;
  await fixture.commitCi(`held:\n  tags: [${hold}]\n  script: [echo held]\n`);
  await pipeline(fixture);
  await fixture.createFleet(hold, 0, 0);
  await until(`${hold}: positive demand proven`, () => demandRow(fixture, hold)?.total_assigned_jobs === 1, 240_000);

  const cases = [
    { name: "pollFailure", short: "poll", drifts: false,
      inject: () => proxy.arm({ kind: "graphql", mode: "reject", limit: Infinity }), restore: () => proxy.release() },
    { name: "anchorDeleted", short: "deleted", drifts: true,
      async inject(keys) {
        // Recreation is refused until restore, so the anchor stays missing.
        proxy.arm({ kind: "register", mode: "reject", role: "anchor", limit: Infinity });
        for (const key of keys) for (const anchor of await anchors(fixture, key)) {
          await fixture.gitlab(`/runners/${anchor.id}`, { method: "DELETE", expected: 204 });
        }
      },
      restore: () => proxy.release() },
    { name: "anchorUnpaused", short: "unpaused", drifts: true,
      async inject(keys) {
        proxy.arm({ kind: "register", mode: "reject", role: "anchor", limit: Infinity });
        for (const key of keys) for (const anchor of await anchors(fixture, key)) {
          await fixture.gitlab(`/runners/${anchor.id}`, { method: "PUT", body: { paused: false } });
        }
      },
      restore: () => proxy.release() },
    // Maintainer still reads runners, but cannot update one: GraphQL then
    // reports updateRunner=false (and may report a zero count).
    { name: "permissionRevoked", short: "permission", drifts: false,
      inject: () => setTokenRole(fixture, 40), restore: () => setTokenRole(fixture, 50) },
  ];
  for (const c of cases) {
    progress.phase = c.name;
    console.log(`Running ${c.name}`);
    const key = `${fixture.prefix}-${c.short}`;
    const keys = [key, hold];
    await fixture.createFleet(key, 1, 1);
    const idle = await runningGeneration(fixture, key, "Idle");
    await until(`${c.name}: both Fleets Ready on proven demand`, async () =>
      (await phases(fixture, keys)).every(p => p === "Ready") && demandRow(fixture, key), 180_000);
    const before = Object.fromEntries(await Promise.all(keys.map(async k => [k, (await anchors(fixture, k)).map(a => a.id)])));
    assert(keys.every(k => before[k].length === 1), `${c.name}: exactly one anchor per Fleet`);

    await c.inject(keys);
    await until(`${c.name}: demand marked stale`, async () => (await phases(fixture, keys)).every(p => p === "Degraded"), 120_000);
    const frozen = Object.fromEntries(keys.map(k => [k, demandRow(fixture, k)]));
    const result = { lastError: Object.fromEntries(await Promise.all(keys.map(async k => [k, (await status(fixture, k)).lastError]))) };
    if (c.name === "anchorUnpaused") {
      // Unpaused is drift: the anchor is replaced, never re-paused in place.
      await until(`${c.name}: unpaused anchors deleted, not re-paused`, async () => {
        for (const k of keys) for (const id of before[k]) {
          const response = await fetch(`${fixture.instance}/api/v4/runners/${id}`, { headers: { "PRIVATE-TOKEN": fixture.adminToken } });
          await response.body?.cancel();
          if (response.status !== 404) return false;
        }
        return true;
      }, 120_000);
    }
    const passes = proxy.gate.counts.inventory ?? 0;
    // A proven pass would now retire the Idle Generation (old revision, target 0).
    await setCapacity(fixture, key, 0, 1);
    result.heldPasses = await steady(async () => {
      const generation = (await generations(fixture, key)).find(g => g.id === idle.generation.id);
      assert.equal(generation?.state, "Idle", `${c.name}: stale demand must not retire the Idle Generation`);
      assert.deepEqual(await containers(fixture, `shaula.generation=${idle.generation.id}`), [idle.container], `${c.name}: container retained`);
      assert.equal((await named(fixture, idle.generation.runner_name)).length, 1, `${c.name}: registration retained`);
      assert.deepEqual(await phases(fixture, keys), ["Degraded", "Degraded"], `${c.name}: staleness stays visible`);
    }, 50_000);
    assert((proxy.gate.counts.inventory ?? 0) >= passes + 4, `${c.name}: the daemon kept reconciling both Fleets`);
    for (const k of keys) assert.deepEqual(demandRow(fixture, k), frozen[k], `${c.name}: ${k} demand snapshot neither replaced nor refreshed`);
    if (c.name === "anchorDeleted") {
      assert(keys.every(k => proxy.posts(`${k}-anchor-r`).some(r => r.fault === "reject")), `${c.name}: Shaula attempted a new anchor`);
    }
    result.fleetCodes = await diagnosticCodes(fixture, "fleets", key);

    await c.restore();
    await until(`${c.name}: demand proven again`, () => keys.every(k => demandRow(fixture, k)?.updated_at > frozen[k].updated_at), 180_000);
    assert.equal(demandRow(fixture, hold).total_assigned_jobs, 1, `${c.name}: the pending job is still counted`);
    await until(`${c.name}: occupancy 0 after the proven scale-down`, async () => (await status(fixture, key)).capacity.occupancy === 0, 600_000);
    await reclaimed(fixture, key, idle);
    // Exactly one paused anchor for each current revision; replaced anchors are gone.
    await until(`${c.name}: anchors converged`, async () => {
      for (const k of keys) {
        const revision = (await status(fixture, k)).desiredRevision;
        const current = await anchors(fixture, k);
        if (current.length !== 1 || current[0].description !== `${k}-anchor-r${revision}` || !current[0].paused) return false;
        if ((k === key || c.drifts) && before[k].includes(current[0].id)) return false;
      }
      return true;
    }, 180_000);
    for (const k of keys.filter(k => k === key || c.drifts)) {
      for (const id of before[k]) await fixture.gitlab(`/runners/${id}`, { expected: 404 });
    }
    await retired(fixture, key);
    checks[c.name] = { ...result, snapshotKept: true, idleRetainedWhileStale: true, retiredOnceProven: true,
      anchorsReplaced: c.drifts ? "both Fleets" : "revision change only" };
  }
  progress.phase = "retire-hold";
  await retired(fixture, hold);
  assert(await fleetGone(fixture, hold));
}
