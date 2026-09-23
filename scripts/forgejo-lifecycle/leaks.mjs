// Keep actual credentials in memory. Assertions expose only the inspected surface.
import assert from "node:assert/strict";
import { readFile, stat } from "node:fs/promises";
import { join } from "node:path";
import { command } from "./support.mjs";

export async function captureCredentials(fixture, observed) {
  const token = fixture.platform === "kubernetes"
    ? JSON.parse(await fixture.kube("get", "secret", observed.id, "-o", "json")).data.token
    : await fixture.cli("exec", observed.id, "cat", "/data/.forgejo-token");
  const runner = fixture.platform === "kubernetes" ? Buffer.from(token, "base64").toString() : token;
  assert(runner.trim().length >= 20, "nonempty real runner credential required for leak scan");
  return [fixture.token, runner.trim()];
}
function clean(body, secrets, surface) {
  for (const secret of secrets) {
    for (const needle of [secret, Buffer.from(secret).toString("base64")]) {
      assert(!body.includes(needle), `credential leak in ${surface}`);
    }
  }
}
async function privateFile(path) {
  assert.equal((await stat(path)).mode & 0o077, 0, "Terraform evidence must be owner-only");
}
// Operator-approved Kubernetes boundary: refresh can retain the single Runner
// token in protected Secret data in a Destroy plan/state. Never permit PATs,
// input variables, resource metadata, command/env, or log projections here.
export function protectedState(body, fixture, observed, secrets) {
  clean(body, [secrets[0]], "Terraform management credential boundary");
  if (fixture.platform !== "kubernetes") {
    clean(body, secrets, "Terraform plan/state");
    return 0;
  }
  const parsed = JSON.parse(body);
  let copies = 0;
  function secretData(value) {
    if (!value?.data?.token) return;
    const metadata = value.metadata?.[0];
    assert(metadata?.name === observed.id && metadata.namespace === fixture.prefix && metadata.uid === observed.secretUid,
      "retained bootstrap Secret must have the original identity");
    assert(value.data.token === secrets[1], "unexpected credential in retained Secret data");
    value.data.token = "[single-runner credential in protected state]";
    copies++;
  }
  function walk(value) {
    if (!value || typeof value !== "object") return;
    if (value.type === "kubernetes_secret_v1" && value.name === "bootstrap") {
      secretData(value.values);
      secretData(value.change?.before);
      secretData(value.change?.after);
      for (const instance of value.instances || []) secretData(instance.attributes);
    }
    for (const child of Object.values(value)) walk(child);
  }
  walk(parsed);
  clean(JSON.stringify(parsed), secrets, "Terraform fields outside protected bootstrap Secret data");
  return copies;
}
export async function leakScan(fixture, key, observed, secrets, alive) {
  let surfaces = 0;
  const inspect = (body, label) => { clean(body, secrets, label); surfaces++; };
  const generations = await fixture.api(`/generations?fleet_key=${key}`);
  inspect(JSON.stringify(generations), "Generation API");
  assert.equal(generations.items.length, 1);
  const id = generations.items[0].id;
  inspect(JSON.stringify(await fixture.api(`/generations/${id}`)), "Generation detail API");
  inspect(await readFile(join(fixture.directory, "daemon.log"), "utf8"), "daemon log");
  if (alive && fixture.platform === "docker") {
    inspect(await fixture.cli("inspect", observed.id), "container argv/environment/metadata");
    inspect(await fixture.cli("logs", observed.id), "runner log");
  } else if (alive) {
    inspect(await fixture.kube("get", "pod", observed.id, "-o", "json"), "Pod argv/environment/metadata");
    const secret = JSON.parse(await fixture.kube("get", "secret", observed.id, "-o", "json"));
    inspect(JSON.stringify(secret.metadata), "Secret metadata");
    assert.equal(secret.immutable, true, "bootstrap Secret frozen after identity-checked release");
    inspect(await fixture.kube("logs", observed.id), "runner log");
  }
  const workspace = join(fixture.directory, "data", "runners", key, id);
  inspect(await readFile(join(workspace, "shaula.tfvars.json"), "utf8"), "Terraform inputs");
  await privateFile(workspace);
  await privateFile(join(workspace, "tfplan"));
  await privateFile(join(workspace, "terraform.tfstate"));
  // Decode the real saved binary plan rather than grepping compressed bytes.
  let protectedTokenCopies = protectedState(await command(fixture.terraform, ["show", "-json", "tfplan"], { cwd: workspace }), fixture, observed, secrets);
  protectedTokenCopies += protectedState(await readFile(join(workspace, "terraform.tfstate"), "utf8"), fixture, observed, secrets);
  surfaces += 2;
  const backup = join(workspace, "terraform.tfstate.backup");
  try {
    const body = await readFile(backup, "utf8");
    await privateFile(backup);
    protectedTokenCopies += protectedState(body, fixture, observed, secrets);
    surfaces++;
  } catch (error) { if (error.code !== "ENOENT") throw error; }
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
  return { surfaces, protectedTokenCopies };
}
