// Real HTTP, read-only SQLite and the actual embedded UI; no response mocking.
import assert from "node:assert/strict";
import { DatabaseSync } from "node:sqlite";
import { writeFile, mkdir } from "node:fs/promises";
import { join } from "node:path";
import { createRequire } from "node:module";
import { until } from "./support.mjs";

export function ledger(fixture, key) {
  const db = new DatabaseSync(join(fixture.directory, "data", "shaula.db"), { readOnly: true });
  try {
    db.exec("BEGIN");
    return {
      fleet: db.prepare("SELECT key,incarnation,desired_revision,observed_revision,phase FROM fleets WHERE key=?").get(key),
      revisions: db.prepare("SELECT revision,template_profile_key,template_revision,template_artifact_digest FROM fleet_revisions WHERE fleet_key=? ORDER BY revision").all(key),
      generations: db.prepare("SELECT id,fleet_key,fleet_revision,template_revision,state,subphase,provisioned_at,expiry_requested_at,resources_destroyed_at FROM runner_generations WHERE fleet_key=? ORDER BY created_at").all(key),
      operations: db.prepare("SELECT o.id,o.generation_id,o.kind,o.state,o.attempts FROM runner_operations o JOIN runner_generations g ON g.id=o.generation_id WHERE g.fleet_key=? ORDER BY o.created_at").all(key),
      invocations: db.prepare("SELECT record_json FROM operation_log_invocations WHERE fleet_key=? ORDER BY ordinal").all(key).map(row => {
        const r = JSON.parse(row.record_json);
        return { id: r.id, generationId: r.generation_id, operation: r.operation, outcome: r.execution_outcome,
          commands: r.commands.map(c => ({ phase: c.phase, exitCode: c.exit_code, effectAttemptId: c.effect_attempt_id })) };
      }),
    };
  } finally { db.close(); }
}

export const hasReason = (report, code) => report.questions.some(q => q.reasons.some(r => r.code === code));
export async function diagnostic(fixture, kind, key, code) {
  return until(`${kind} diagnostic: ${code}`, async () => {
    const result = await fixture.api(`/${kind}s/${key}/diagnostics`);
    return hasReason(result, code) && result;
  });
}

export class Evidence {
  constructor(fixture) { this.fixture = fixture; this.receipts = []; this.secrets = []; }
  async start() {
    const require = createRequire(new URL("../../web/package.json", import.meta.url));
    const { chromium } = require("@playwright/test");
    this.browser = await chromium.launch({ headless: true });
    this.context = await this.browser.newContext({ viewport: { width: 1440, height: 1000 }, ignoreHTTPSErrors: true });
    this.secrets.push(this.fixture.token, this.fixture.oidc.token);
    this.directory = join(this.fixture.directory, "public-evidence");
    await mkdir(this.directory, { mode: 0o700 });
    const page = await this.context.newPage();
    try {
      const session = page.waitForResponse(r => new URL(r.url()).pathname === "/api/v1/session");
      await page.goto(`${this.fixture.publicUrl}/fleets`);
      assert.equal((await session).status(), 200, "real browser OIDC session required");
    } finally { await page.close(); }
  }
  clean(value) {
    const text = JSON.stringify(value);
    for (const secret of this.secrets) {
      for (const needle of [secret, Buffer.from(secret).toString("base64")]) {
        assert(!text.includes(needle), "credential detected in acceptance evidence");
      }
    }
    return value;
  }
  async capture(name, fleet, kind, key, code) {
    const f = this.fixture;
    const page = await this.context.newPage();
    try {
      const endpoint = `/api/v1/${kind}s/${key}/diagnostics`;
      const response = page.waitForResponse(r => new URL(r.url()).pathname === endpoint && r.status() === 200);
      await page.goto(`${f.publicUrl}${kind === "fleet" ? `/fleets/${key}` : `/jobs/runners/${key}`}`);
      const httpResponse = await response;
      assert.match(httpResponse.headers()["cache-control"], /private.*no-store/);
      let report = await httpResponse.json();
      if (!hasReason(report, code)) report = await diagnostic(f, kind, key, code);
      const panel = page.getByRole("region", { name: "Why", exact: true });
      await panel.waitFor();
      await until("real UI renders diagnostic reason", async () => {
        await panel.locator("details").evaluateAll(nodes => nodes.forEach(n => { n.open = true; }));
        return (await panel.innerText()).includes(code);
      }, 35_000);
      // Read the real DOM and compare with the real diagnostics API in the same interval.
      const uiText = await panel.innerText();
      report = await f.api(`/${kind}s/${key}/diagnostics`);
      assert(hasReason(report, code), "HTTP reason must remain present during UI capture");
      const domain = ledger(f, fleet);
      const status = await f.api(`/fleets/${fleet}/status`);
      const runners = (await f.forgejo("/admin/actions/runners")).filter(r => r.name.startsWith(`${fleet}-`))
        .map(({ id, name, status, ephemeral, labels }) => ({ id, name, status, ephemeral, labels }));
      const containers = (await f.cli("ps", "-a", "--no-trunc", "-q", "--filter", `label=shaula.fleet=${fleet}`)).split("\n").filter(Boolean);
      const receipt = this.clean({ name, capturedAt: new Date().toISOString(), expectedReason: code, http: report,
        uiText, ledger: domain, capacity: status.capacity, provider: { runners, containers } });
      await writeFile(join(this.directory, `${name}.json`), JSON.stringify(receipt, null, 2), { mode: 0o600 });
      await panel.screenshot({ path: join(this.directory, `${name}.png`) });
      this.receipts.push(name);
      return receipt;
    } finally { await page.close(); }
  }
  async close() { await this.browser?.close(); }
}
