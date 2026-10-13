// Real-GitLab E2E for crates/shaula-gitlab. This script only provisions a
// disposable GitLab (group, project, CI file, Owner group access token); the
// Rust test drives every adapter call and starts the run-single runner.
//
//   CONTAINER_ENGINE=podman node scripts/gitlab-pool-spike/adapter-e2e.mjs > results.json
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { Fixture, runnerImage, serverImage } from "./support.mjs";

const tag = "shaula-e2e";
const ci = `match:
  tags: [${tag}]
  script:
    - echo "TASK_EXECUTED job=$CI_JOB_ID runner=$CI_RUNNER_ID"
    - sleep 15
mismatch:
  tags: [${tag}, gpu]
  script: [echo must-stay-pending]
`;

const fixture = new Fixture();
let artifact;
try {
  await fixture.start();
  const group = (await fixture.api("/groups", {
    method: "POST",
    body: { name: "fleet", path: "fleet", visibility: "private" },
    expected: 201,
  })).body;
  const project = (await fixture.api("/projects", {
    method: "POST",
    body: { name: "app", namespace_id: group.id, visibility: "private", initialize_with_readme: true },
    expected: 201,
  })).body;
  const expires = new Date(Date.now() + 2 * 86_400_000).toISOString().slice(0, 10);
  const operator = (await fixture.api(`/groups/${group.id}/access_tokens`, {
    method: "POST",
    body: { name: "shaula", scopes: ["create_runner", "manage_runner", "read_api"], access_level: 50, expires_at: expires },
    expected: 201,
  })).body.token;
  await fixture.api(`/projects/${project.id}/repository/commits`, {
    method: "POST",
    body: {
      branch: "main",
      commit_message: "ci [skip ci]",
      actions: [{ action: "create", file_path: ".gitlab-ci.yml", content: ci }],
    },
    expected: 201,
  });

  // Credentials travel as 0600 files inside the fixture directory, never env/argv.
  const secret = async (name, value) => {
    const path = join(fixture.directory, name);
    await writeFile(path, value, { mode: 0o600, flag: "wx" });
    return path;
  };
  const config = {
    url: fixture.url,
    internal_url: fixture.internalUrl,
    group_path: group.full_path,
    group_id: group.id,
    project_id: project.id,
    tag,
    operator_token_file: await secret("operator-token", operator),
    admin_token_file: await secret("admin-token", fixture.adminToken),
    engine: fixture.engine,
    network: fixture.network,
    runner_image: runnerImage,
    work_dir: fixture.directory,
    artifact: join(fixture.directory, "artifact.json"),
  };
  const configPath = await secret("e2e.json", JSON.stringify(config));
  const code = await new Promise((resolve) => {
    const child = spawn(
      "cargo",
      ["test", "-p", "shaula-gitlab", "--test", "live_gitlab", "--", "--ignored", "--nocapture"],
      { stdio: ["ignore", process.stderr, "inherit"], env: { ...process.env, SHAULA_GITLAB_E2E: configPath } },
    );
    child.on("exit", resolve);
  });
  assert.equal(code, 0, "live adapter test failed");
  artifact = JSON.parse(await readFile(config.artifact, "utf8"));
  artifact.images = { server: serverImage, runner: runnerImage };
} finally {
  await fixture.close();
}
// Only emitted after the Rust test passed and teardown succeeded.
process.stdout.write(`${JSON.stringify(artifact, null, 2)}\n`);
