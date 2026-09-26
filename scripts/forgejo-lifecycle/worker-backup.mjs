// Offline full-set restoration on a disposable instance. Never roll production
// authority backward or discard the live directory in order to make this pass.
import assert from "node:assert/strict";
import { cp, mkdir, readFile, readdir, readlink, rename, writeFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { join, relative } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { workers } from "./worker-pressure.mjs";
import { until } from "./support.mjs";

async function manifest(root, path = root, entries = []) {
  for (const entry of await readdir(path, { withFileTypes: true })) {
    const full = join(path, entry.name);
    const name = relative(root, full);
    if (entry.isDirectory()) await manifest(root, full, entries);
    else if (entry.isSymbolicLink()) entries.push([name, "link", await readlink(full)]);
    else {
      assert(entry.isFile(), "backup cannot silently skip a special file");
      entries.push([name, "sha256", createHash("sha256").update(await readFile(full)).digest("hex")]);
    }
  }
  return entries.sort((a, b) => a[0].localeCompare(b[0]));
}

export async function workerBackup(fixture) {
  const key = `${fixture.prefix}-backup`;
  await fixture.createFleet(key, 1);
  const observed = await fixture.observe(key, "idle");
  await fixture.capZero(key);
  const original = workers(fixture, key)[0];
  const identity = JSON.parse(original.process_identity);
  const creates = fixture.dockerProxy.gate.containerCreates;
  const registrations = fixture.registrationProxy.gate.registrationPosts;
  await fixture.stopDaemon();
  await until("all original writers fenced before offline backup", async () =>
    (await readFile(`${identity.containment}/cgroup.events`, "utf8")).includes("populated 0"));
  const data = join(fixture.directory, "data");
  const db = new DatabaseSync(join(data, "shaula.db"));
  try {
    assert.equal(db.prepare("PRAGMA quick_check").get().quick_check, "ok");
    db.exec("PRAGMA wal_checkpoint(TRUNCATE)");
  } finally { db.close(); }
  const directory = join(fixture.directory, "offline-backup");
  await mkdir(directory, { mode: 0o700 });
  await cp(data, join(directory, "data"), { recursive: true, dereference: false, verbatimSymlinks: true });
  await cp(join(fixture.directory, "config.json"), join(directory, "config.json"));
  const files = await manifest(data);
  assert.deepEqual(await manifest(join(directory, "data")), files, "backup contains all state, inputs, artifacts and emergency evidence");
  assert.deepEqual(await readFile(join(directory, "config.json")), await readFile(join(fixture.directory, "config.json")));
  await writeFile(join(directory, "runtime.json"), JSON.stringify(fixture.runtimeTuple), { mode: 0o600 });
  // No daemon has run since the checkpoint. Recheck that no late local writer
  // changed authority, and that the exact external resource is still present.
  assert.deepEqual(await manifest(data), files);
  assert.equal(await fixture.cli("ps", "-a", "--no-trunc", "-q", "--filter", `label=shaula.fleet=${key}`), observed.id);
  await rename(data, join(fixture.directory, "preserved-before-restore"));
  await cp(join(directory, "data"), data, { recursive: true, dereference: false, verbatimSymlinks: true });
  assert.deepEqual(await manifest(data), files, "exact restored set verified before starting any writer");
  await fixture.startDaemon();
  assert(workers(fixture, key)[0].worker_epoch > original.worker_epoch, "restore must rotate the fenced epoch");
  await fixture.queue(key, 8);
  await fixture.observe(key, "active");
  await fixture.reclaimed(key, observed);
  assert.equal(fixture.dockerProxy.gate.containerCreates, creates, "restore never repeats Create");
  assert.equal(fixture.registrationProxy.gate.registrationPosts, registrations, "restore never repeats registration");
  return { completeSetFiles: files.length, sqliteQuickCheck: "ok", originalWriters: "fenced before copy",
    manifestVerified: true, originalDirectory: "preserved", restoredEpoch: workers(fixture, key)[0].worker_epoch,
    duplicateCreates: 0, duplicateRegistrations: 0, cleanup: "Destroyed; occupancy zero; resource and registration absent",
    scope: "quiescent same-host full-set restore; post-checkpoint divergent authority is not rolled back" };
}
