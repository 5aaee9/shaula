// Credentials stay in memory; assertions name only the inspected surface.
// The GitLab registration token must exist only in the copied config.toml.
import assert from "node:assert/strict";
import { readFile, stat } from "node:fs/promises";
import { join } from "node:path";
import { DatabaseSync } from "node:sqlite";

function clean(body, secrets, surface) {
  for (const secret of secrets) {
    for (const needle of [secret, Buffer.from(secret).toString("base64")]) {
      assert(!body.includes(needle), `credential leak in ${surface}`);
    }
  }
}

// `adjacent` is the short token the runner itself logs (spec 0043 G8): it may
// appear only in the runner log, never on a Shaula-owned surface.
export async function leakScan(fixture, key, observed, secrets, alive, adjacent = []) {
  let surfaces = 0;
  const inspect = (body, label, own = true) => { clean(body, own ? [...secrets, ...adjacent] : secrets, label); surfaces++; };
  const generations = await fixture.api(`/generations?fleet_key=${key}`);
  inspect(JSON.stringify(generations), "Generation API");
  const id = observed.generation;
  inspect(JSON.stringify(await fixture.api(`/generations/${id}`)), "Generation detail API");
  inspect(JSON.stringify(await fixture.api(`/fleets/${key}/status`)), "Fleet status API");
  inspect(await readFile(join(fixture.directory, "daemon.log"), "utf8"), "daemon log");
  if (alive) {
    inspect(await fixture.cli("inspect", observed.id), "container argv/environment/metadata");
    // The runner prints a 9-character token prefix; the full token must not appear.
    inspect(await fixture.logs(observed.id), "runner log", false);
  }
  const database = join(fixture.directory, "data", "shaula.db");
  assert.equal((await stat(database)).mode & 0o077, 0, "control-plane database must be owner-only");
  const db = new DatabaseSync(database, { readOnly: true });
  let durable;
  try {
    durable = db.prepare("SELECT s.state_bytes, w.protected_input FROM generation_http_state s JOIN lifecycle_workers w ON w.generation_id = s.generation_id WHERE s.generation_id = ?").get(id);
  } finally { db.close(); }
  assert(durable?.state_bytes && durable.protected_input, "authoritative HTTP state and original input required");
  inspect(Buffer.from(durable.protected_input).toString(), "retained Terraform inputs");
  inspect(Buffer.from(durable.state_bytes).toString(), "Terraform state");
  let cursor;
  do {
    const page = await fixture.api(`/generations/${id}/invocations${cursor ? `?cursor=${encodeURIComponent(cursor)}` : ""}`);
    inspect(JSON.stringify(page), "invocation API");
    for (const invocation of page.items) {
      let logCursor;
      do {
        const logs = await fixture.api(`/invocations/${invocation.id}/logs?limit_bytes=1048576${logCursor ? `&cursor=${encodeURIComponent(logCursor)}` : ""}`);
        inspect(JSON.stringify(logs), "Operation Log API");
        logCursor = logs.next_cursor;
      } while (logCursor);
    }
    cursor = page.next_cursor;
  } while (cursor);
  return { surfaces };
}
