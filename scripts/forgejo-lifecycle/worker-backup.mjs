// Offline full-set restoration on a disposable instance. Never roll production
// authority backward or discard the live directory in order to make this pass.
import assert from "node:assert/strict";
import { cp, mkdir, readFile, readdir, readlink, rename, writeFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { join, relative } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { workers } from "./worker-pressure.mjs";
import { fenceFixture } from "./worker-fence.mjs";
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

const commitment = files => createHash("sha256").update(JSON.stringify(files)).digest("hex");

async function restoreUnchanged(fixture, backup, preserved) {
  const data = join(fixture.directory, "data");
  const current = commitment(await manifest(data));
  const checkpoint = commitment(await manifest(join(backup, "data")));
  if (current !== checkpoint) throw new Error("RestoreBlockedNewerEvidence");
  await rename(data, join(fixture.directory, preserved));
  await cp(join(backup, "data"), data, { recursive: true, dereference: false, verbatimSymlinks: true });
  assert.equal(commitment(await manifest(data)), checkpoint, "restored complete set verified before any writer starts");
}

export async function workerBackup(fixture) {
  const key = `${fixture.prefix}-backup`;
  await fixture.createFleet(key, 1);
  const observed = await fixture.observe(key, "idle");
  await until("original Create is durably settled before backup", () => workers(fixture, key)[0]?.state === "Idle");
  console.log("Backup checkpoint: original idle resource observed");
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
  assert.equal(commitment(await manifest(join(directory, "data"))), commitment(files), "backup contains all state, inputs, artifacts and emergency evidence");
  assert((await readFile(join(directory, "config.json"))).equals(await readFile(join(fixture.directory, "config.json"))), "matching private configuration");
  await writeFile(join(directory, "runtime.json"), JSON.stringify(fixture.runtimeTuple), { mode: 0o600 });
  // No daemon has run since the checkpoint. Recheck that no late local writer
  // changed authority, and that the exact external resource is still present.
  assert.equal(commitment(await manifest(data)), commitment(files));
  assert.equal(await fixture.cli("ps", "-a", "--no-trunc", "-q", "--filter", `label=shaula.fleet=${key}`), observed.id);
  await restoreUnchanged(fixture, directory, "preserved-before-restore");
  console.log("Backup checkpoint: complete set restored with original writers stopped");
  await fixture.startDaemon();
  await until("restore rotates the fenced epoch", () => workers(fixture, key)[0].worker_epoch > original.worker_epoch);
  console.log("Backup checkpoint: fenced recovery epoch observed");
  await fixture.queue(key, 40, false, [key], false);
  await fixture.observe(key, "active");
  await fixture.reclaimed(key, observed);
  assert.equal(fixture.dockerProxy.gate.containerCreates, creates, "restore never repeats Create");
  assert.equal(fixture.registrationProxy.gate.registrationPosts, registrations, "restore never repeats registration");
  return { completeSetFiles: files.length, sqliteQuickCheck: "ok", originalWriters: "fenced before copy",
    manifestVerified: true, originalDirectory: "preserved", restoredEpoch: workers(fixture, key)[0].worker_epoch,
    duplicateCreates: 0, duplicateRegistrations: 0, cleanup: "Destroyed; occupancy zero; resource and registration absent",
    scope: "quiescent same-host full-set restore; post-checkpoint divergent authority is not rolled back" };
}

export async function backupDivergence(fixture) {
  const creates = fixture.dockerProxy.gate.containerCreates;
  await fixture.stopDaemon();
  await fenceFixture(fixture);
  const data = join(fixture.directory, "data");
  const backup = join(fixture.directory, "before-late-effect");
  await mkdir(backup, { mode: 0o700 });
  await cp(data, join(backup, "data"), { recursive: true, dereference: false, verbatimSymlinks: true });
  const old = commitment(await manifest(join(backup, "data")));
  await fixture.startDaemon();
  const key = `${fixture.prefix}-post-checkpoint`;
  await fixture.createFleet(key, 1);
  const observed = await fixture.observe(key, "idle");
  await until("late Create is durably settled before fencing", () => workers(fixture, key)[0]?.state === "Idle");
  await fixture.capZero(key);
  await fixture.stopDaemon();
  await fenceFixture(fixture);
  const newer = commitment(await manifest(data));
  assert.notEqual(newer, old, "the real post-checkpoint Create changed authority");
  await assert.rejects(restoreUnchanged(fixture, backup, "must-not-replace-newer"), /RestoreBlockedNewerEvidence/);
  assert.equal(commitment(await manifest(data)), newer, "newer SQLite/material evidence remains byte-exact");
  assert.equal(commitment(await manifest(join(backup, "data"))), old, "old checkpoint is preserved separately");
  assert.equal(await fixture.cli("ps", "-a", "--no-trunc", "-q", "--filter", `label=shaula.fleet=${key}`), observed.id);
  await fixture.startDaemon();
  await fixture.queue(key, 40, false, [key], false);
  await fixture.observe(key, "active");
  await fixture.reclaimed(key, observed);
  assert.equal(fixture.dockerProxy.gate.containerCreates, creates + 1, "recovery never replays the post-checkpoint Create");
  return { oldCheckpoint: old, retainedAuthority: newer, lateCreates: 1,
    rollback: "refused before replacing any live file", recovery: "current authority resumed; exact resource reclaimed" };
}
