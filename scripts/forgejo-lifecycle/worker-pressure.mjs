// Small-host acceptance, not a claim that 1,024 workers fit this machine.
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { setTimeout as delay } from "node:timers/promises";
import { until } from "./support.mjs";

export function workers(fixture, key) {
  const db = new DatabaseSync(join(fixture.directory, "data", "shaula.db"), { readOnly: true });
  try {
    return db.prepare(`SELECT w.phase,w.process_identity,w.cleanup_only,s.worker_epoch,g.id,g.state
      FROM lifecycle_workers w JOIN runner_generations g ON g.id=w.generation_id
      JOIN generation_http_state s ON s.generation_id=g.id WHERE g.fleet_key=?`).all(key);
  } finally { db.close(); }
}

async function usage(pid) {
  const status = await readFile(`/proc/${pid}/status`, "utf8");
  return { rssKiB: Number(status.match(/^VmRSS:\s+(\d+)/m)?.[1]),
    threads: Number(status.match(/^Threads:\s+(\d+)/m)?.[1]) };
}

export async function workerPressure(fixture) {
  fixture.config.lifecycle.max_workers = 4;
  fixture.config.lifecycle.recovery_reserve = 1;
  fixture.config.execution.create_concurrency = 1;
  fixture.config.execution.destroy_concurrency = 1;
  await fixture.restart(900);
  const keys = Array.from({ length: 6 }, (_, i) => `${fixture.prefix}-pressure-${i}`);
  for (const key of keys) await fixture.createFleet(key, 1);
  const admitted = await until("three waiting workers fill the new-admission budget", async () => {
    const rows = (await fixture.forgejo("/admin/actions/runners")).filter(r => keys.some(k => r.name.startsWith(`${k}-`)));
    return rows.length === 3 && rows.every(r => r.status === "idle") && rows;
  });
  // One mutation permit must still let all three workers finish Create and wait.
  const liveKeys = keys.filter(key => admitted.some(r => r.name.startsWith(`${key}-`)));
  const samples = [];
  const latencies = [];
  for (let i = 0; i < 20; i++) {
    const rows = keys.flatMap(key => workers(fixture, key));
    assert.equal(rows.length, 3, "saturation must not allocate an unbounded durable queue");
    const sample = await Promise.all(rows.map(row => usage(JSON.parse(row.process_identity).process_id)));
    assert(sample.every(s => s.threads <= 3 && s.rssKiB > 0 && s.rssKiB < 256 * 1024), "waiting worker resource bound");
    samples.push(sample);
    const start = performance.now();
    await Promise.all(keys.map(key => fixture.api(`/fleets/${key}/status`)));
    latencies.push(performance.now() - start);
    await delay(250);
  }
  // Pending fleets must stop competing before a recovery slot is exercised.
  for (const key of keys) await fixture.capZero(key);
  const old = workers(fixture, liveKeys[0])[0];
  const identity = JSON.parse(old.process_identity);
  process.kill(identity.process_id, "SIGKILL");
  // The daemon must perform recovery while the other waiting workers remain.
  await fixture.queue(liveKeys[0], 8);
  const first = await fixture.observe(liveKeys[0], "active");
  await fixture.reclaimed(liveKeys[0], first);
  assert(workers(fixture, liveKeys[0])[0].worker_epoch > old.worker_epoch, "cleanup must use a new fenced epoch");
  for (const key of liveKeys.slice(1)) {
    await fixture.queue(key, 8);
    const observed = await fixture.observe(key, "active");
    await fixture.reclaimed(key, observed);
  }
  for (const key of keys.filter(key => !liveKeys.includes(key))) {
    assert.equal(workers(fixture, key).length, 0, "pending demand cannot escape its zero-capacity update");
  }
  fixture.config.lifecycle.max_workers = 8;
  fixture.config.lifecycle.recovery_reserve = 2;
  fixture.config.execution.create_concurrency = 8;
  fixture.config.execution.destroy_concurrency = 8;
  await fixture.restart(300);
  return { maxWorkers: 4, recoveryReserve: 1, competingFleets: 6, admittedWorkers: 3,
    mutationPermits: 1, samples: samples.length,
    maxWorkerRssKiB: Math.max(...samples.flat().map(s => s.rssKiB)),
    maxWorkerThreads: Math.max(...samples.flat().map(s => s.threads)),
    maxSixStatusReadsMs: Math.ceil(Math.max(...latencies)),
    recoveryAtSaturation: "passed", scope: "small-host saturation; not maximum-count or log-flood acceptance" };
}
