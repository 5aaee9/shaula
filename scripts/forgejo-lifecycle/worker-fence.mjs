// Teardown only: this is never a successful resource-reclamation receipt.
import assert from "node:assert/strict";
import { readFile, realpath, writeFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { dirname, basename, join } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { until } from "./support.mjs";

export async function fenceFixture(fixture) {
  if (!fixture.directory) return;
  const path = join(fixture.directory, "data", "shaula.db");
  if (!existsSync(path)) return;
  const db = new DatabaseSync(path, { readOnly: true });
  let rows;
  try {
    if (!db.prepare("SELECT name FROM sqlite_master WHERE name='lifecycle_workers'").get()) return;
    rows = db.prepare("SELECT process_identity FROM lifecycle_workers WHERE process_identity IS NOT NULL").all();
  } finally { db.close(); }
  const group = (await readFile("/proc/self/cgroup", "utf8")).split("\n").find(line => line.startsWith("0::/"))?.slice(3);
  assert(group && group !== "/", "a delegated service subtree is required");
  const root = await realpath(process.env.SHAULA_TEST_CGROUP || `/sys/fs/cgroup${group}`);
  for (const row of rows) {
    const { containment } = JSON.parse(row.process_identity);
    assert.equal(dirname(containment), root, "only this fixture's delegated child cgroups may be fenced");
    assert.match(basename(containment), /^shaula-[0-9a-f-]{36}$/);
    let canonical;
    try { canonical = await realpath(containment); }
    catch (error) { if (error.code === "ENOENT") continue; throw error; }
    assert.equal(canonical, containment, "no redirected containment");
    await writeFile(join(containment, "cgroup.kill"), "1");
    await until("fixture writers stopped before external teardown", async () =>
      (await readFile(join(containment, "cgroup.events"), "utf8")).includes("populated 0"), 10000);
  }
}
