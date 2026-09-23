// Exact API/scope evidence against the disposable server, never a credential export.
import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import { json, until } from "./support.mjs";

export async function permissions(fixture) {
  const user = "scope-owner";
  const other = "scope-other";
  for (const name of [user, other]) {
    await fixture.cli("exec", "--user", "git", fixture.server, "forgejo", "admin", "user", "create", "--username", name,
      "--random-password", "--email", `${name}@example.invalid`, "--must-change-password=false");
  }
  async function token(username, scopes) {
    return fixture.cli("exec", "--user", "git", fixture.server, "forgejo", "admin", "user", "generate-access-token",
      "--username", username, "--token-name", randomUUID(), "--scopes", scopes, "--raw");
  }
  const ownerToken = await token(user, "all");
  const otherToken = await token(other, "all");
  const api = (credential, path, method, body, expected) => json(`${fixture.direct}/api/v1${path}`, credential, method, body, expected);
  const org = await api(ownerToken, "/orgs", "POST", { username: "scope-org", visibility: "private" }, 201);
  const repo = await api(ownerToken, "/user/repos", "POST", { name: "scope-repo", private: true, auto_init: true }, 201);
  const orgRepo = await api(ownerToken, `/orgs/${org.username}/repos`, "POST", { name: "scope-repo", private: true, auto_init: true }, 201);
  await api(otherToken, "/user/repos", "POST", { name: "scope-repo", private: true, auto_init: true }, 201);
  const results = {};
  async function denied(credential, path, method, body) {
    const response = await fetch(`${fixture.direct}/api/v1${path}`, { method, headers: { authorization: `Bearer ${credential}`, "content-type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body), redirect: "error", signal: AbortSignal.timeout(15_000) });
    await response.arrayBuffer();
    assert([403, 404].includes(response.status), `scoped ${method} must deny unauthorized access (status ${response.status})`);
    return response.status;
  }
  for (const [kind, username, category, path, identity, scope] of [
    ["instance", fixture.user, "admin", "/admin/actions/runners", null, { kind: "instance" }],
    ["user", user, "user", "/user/actions/runners", "/user", { kind: "user" }],
    ["organization", user, "organization", `/orgs/${org.username}/actions/runners`, `/orgs/${org.username}`, { kind: "organization", name: org.username }],
    ["repository", user, "repository", `/repos/${user}/${repo.name}/actions/runners`, `/repos/${user}/${repo.name}`, { kind: "repository", owner: user, name: repo.name }],
  ]) {
    const write = await token(username, `write:${category}`);
    const read = await token(username, `read:${category}`);
    if (identity) await api(write, identity);
    await api(write, `${path}/jobs?labels=permission-check`);
    await api(write, path);
    const registered = await api(write, path, "POST", { name: `${fixture.prefix}-permission-${kind}`, ephemeral: true }, 201);
    assert(registered.id > 0 && registered.token, "registration material returned");
    await api(write, `${path}/${registered.id}`);
    await api(read, path);
    await denied(read, path, "POST", { name: "read-only-denied", ephemeral: true });
    await denied(read, `${path}/${registered.id}`, "DELETE");
    await api(write, `${path}/${registered.id}`, "DELETE", undefined, 204);
    await api(write, `${path}/${registered.id}`, "GET", undefined, 404);
    // Exercise Shaula's public attester, not only direct endpoint calls.
    const key = `permission-${kind}`;
    await fixture.api(`/github-auth-profiles/${key}`, "PUT", { kind: "forgejo_token", instance_url: fixture.direct, scope, token: write }, 202, { "if-none-match": "*" });
    await until(`${kind}: minimum-scope profile Active`, async () => {
      const profile = await fixture.api(`/github-auth-profiles/${key}`);
      assert(!["Rejected", "Unsupported"].includes(profile.status), `${kind}: minimum-scope profile rejected`);
      return profile.activeRevision === 1;
    });
    let history;
    const historyRepo = kind === "organization" ? orgRepo : repo;
    if (category === "repository") {
      await api(write, `/repositories/${historyRepo.id}`);
      await api(write, `/repos/${historyRepo.full_name}/actions/tasks`);
      history = "included";
    } else {
      await denied(write, `/repositories/${historyRepo.id}`, "GET");
      const enriched = await token(username, `write:${category},read:repository`);
      await api(enriched, `/repositories/${historyRepo.id}`);
      await api(enriched, `/repos/${historyRepo.full_name}/actions/tasks`);
      history = "read:repository additionally required; existing repository access also required";
    }
    results[kind] = { management: `write:${category}`, readOnlyMutations: "denied", shaulaAttestation: "Active", taskHistory: history };
  }
  await denied(ownerToken, `/repos/${other}/scope-repo/actions/runners`, "GET");
  await denied(otherToken, `/orgs/${org.username}/actions/runners`, "GET");
  await denied(ownerToken, "/admin/actions/runners", "GET");
  results.unauthorizedOwner = "private repository, organization and instance denied";
  return results;
}
