#!/usr/bin/env node
// Local GitHub App transport. Never send the private key to a remote host.
import { sign } from "node:crypto";
import { constants } from "node:fs";
import { lstat, open } from "node:fs/promises";
import { dirname, isAbsolute } from "node:path";
import { pathToFileURL } from "node:url";

const REPOSITORY = "5aaee9/shaula";
const API = "https://api.github.com";
const ROOT = `/repos/${REPOSITORY}/actions/runners`;
const LABELS = ["self-hosted", "linux", "x64", "shaula-codex-docker"];

class Rejected extends Error {}
function requireThat(condition, reason) {
  if (!condition) throw new Rejected(reason);
}
function identifier(value) {
  return (
    typeof value === "string" && /^[1-9][0-9]*$/.test(value) && Number.isSafeInteger(Number(value))
  );
}
function runnerName(value) {
  return typeof value === "string" && /^[A-Za-z0-9][A-Za-z0-9_.-]{0,99}$/.test(value);
}

export function options(argv) {
  const [mode, ...remaining] = argv;
  const common = ["app-id", "installation-id", "private-key", "repository"];
  const modes = {
    issue: ["name", "output"],
    inspect: ["runner-id"],
    remove: ["runner-id", "expected-name", "generation-id", "container-id", "output"],
  };
  requireThat(Object.hasOwn(modes, mode) && remaining.length % 2 === 0, "arguments_invalid");
  const result = { mode, repository: REPOSITORY };
  const seen = new Set();
  for (let index = 0; index < remaining.length; index += 2) {
    const key = remaining[index].replace(/^--/, "");
    requireThat(
      remaining[index] === `--${key}` &&
        [...common, ...modes[mode]].includes(key) &&
        !seen.has(key) &&
        remaining[index + 1],
      "arguments_invalid",
    );
    seen.add(key);
    result[key] = remaining[index + 1];
  }
  requireThat(
    [...common.filter((key) => key !== "repository"), ...modes[mode]].every(
      (key) => typeof result[key] === "string",
    ),
    "arguments_invalid",
  );
  requireThat(
    identifier(result["app-id"]) &&
      identifier(result["installation-id"]) &&
      result.repository === REPOSITORY &&
      isAbsolute(result["private-key"]),
    "arguments_invalid",
  );
  if (mode === "issue") requireThat(runnerName(result.name), "arguments_invalid");
  else requireThat(identifier(result["runner-id"]), "arguments_invalid");
  if (mode === "remove") {
    requireThat(
      runnerName(result["expected-name"]) &&
        /^[A-Za-z0-9_-]{1,100}$/.test(result["generation-id"]) &&
        /^[a-f0-9]{64}$/.test(result["container-id"]),
      "arguments_invalid",
    );
  }
  if (result.output) requireThat(isAbsolute(result.output), "arguments_invalid");
  return result;
}

async function privateOutput(path) {
  const parent = await lstat(dirname(path));
  requireThat(parent.isDirectory() && !parent.isSymbolicLink(), "output_directory_invalid");
  if (process.platform !== "win32") {
    requireThat(
      parent.uid === process.geteuid() && (parent.mode & 0o077) === 0,
      "output_directory_must_be_private",
    );
  }
  // On Windows the caller must first restrict and verify the parent NTFS ACL.
  // Exclusive creation also blocks accidental issue replay after uncertainty.
  return open(
    path,
    constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | (constants.O_NOFOLLOW ?? 0),
    0o600,
  );
}

async function appJwt(path, appId) {
  const metadata = await lstat(path);
  requireThat(metadata.isFile() && !metadata.isSymbolicLink(), "private_key_file_invalid");
  const handle = await open(path, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  let bytes;
  try {
    const info = await handle.stat();
    requireThat(info.isFile() && info.size > 0 && info.size <= 32768, "private_key_file_invalid");
    if (process.platform !== "win32") {
      requireThat(
        info.uid === process.geteuid() && (info.mode & 0o077) === 0,
        "private_key_file_must_be_private",
      );
    }
    bytes = await handle.readFile();
    const encode = (value) => Buffer.from(JSON.stringify(value)).toString("base64url");
    const now = Math.floor(Date.now() / 1000);
    const message = `${encode({ alg: "RS256", typ: "JWT" })}.${encode({ iat: now - 60, exp: now + 540, iss: appId })}`;
    return `${message}.${sign("RSA-SHA256", Buffer.from(message), bytes).toString("base64url")}`;
  } finally {
    bytes?.fill(0);
    await handle.close();
  }
}

async function request(token, method, path, phase, body, allowed = [200]) {
  let response;
  try {
    response = await fetch(API + path, {
      method,
      redirect: "error",
      signal: AbortSignal.timeout(30000),
      headers: {
        accept: "application/vnd.github+json",
        authorization: `Bearer ${token}`,
        "content-type": "application/json",
        "user-agent": "shaula-docker-smoke",
        "x-github-api-version": "2022-11-28",
      },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch {
    throw new Rejected(`${phase}_transport_outcome_uncertain`);
  }
  if (!allowed.includes(response.status)) {
    await response.body?.cancel();
    throw new Rejected(`${phase}_http_${response.status}`);
  }
  if ([204, 404].includes(response.status)) {
    await response.body?.cancel();
    return { status: response.status, data: null };
  }
  const chunks = [];
  let length = 0;
  try {
    for await (const chunk of response.body) {
      length += chunk.length;
      requireThat(length <= 1024 * 1024, `${phase}_response_too_large`);
      chunks.push(chunk);
    }
    return { status: response.status, data: JSON.parse(Buffer.concat(chunks).toString("utf8")) };
  } catch (error) {
    throw error instanceof Rejected ? error : new Rejected(`${phase}_response_invalid`);
  }
}

async function installationToken(args) {
  const jwt = await appJwt(args["private-key"], args["app-id"]);
  const path = `/app/installations/${args["installation-id"]}`;
  const { data: installation } = await request(jwt, "GET", path, "installation_identity");
  requireThat(
    installation?.app_id === Number(args["app-id"]) &&
      installation.account?.login?.toLowerCase() === "5aaee9" &&
      installation.account?.type === "User",
    "installation_identity_mismatch",
  );
  const { data } = await request(
    jwt,
    "POST",
    `${path}/access_tokens`,
    "installation_token",
    {
      repositories: ["shaula"],
      permissions: { administration: "write", metadata: "read" },
    },
    [201],
  );
  requireThat(
    typeof data?.token === "string" && data.token.length > 0,
    "installation_token_invalid",
  );
  return data.token;
}

function safeRunner(value, expectedId) {
  requireThat(
    Number.isSafeInteger(value?.id) &&
      value.id > 0 &&
      runnerName(value.name) &&
      ["online", "offline"].includes(value.status) &&
      typeof value.busy === "boolean" &&
      (expectedId === undefined || value.id === expectedId),
    "runner_response_invalid",
  );
  return { id: value.id, name: value.name, status: value.status, busy: value.busy };
}

async function absent(token, runnerId) {
  // A runner-specific 404 alone may hide lost authorization. Require a working
  // authenticated repository inventory and verify every page before absence.
  let expectedTotal;
  const seen = new Set();
  for (let page = 1; page <= 100; page++) {
    const { data } = await request(
      token,
      "GET",
      `${ROOT}?per_page=100&page=${page}`,
      "runner_inventory",
    );
    requireThat(
      Array.isArray(data?.runners) &&
        data.runners.length <= 100 &&
        Number.isSafeInteger(data.total_count) &&
        data.total_count >= 0 &&
        data.total_count <= 10000,
      "runner_inventory_invalid",
    );
    expectedTotal ??= data.total_count;
    requireThat(data.total_count === expectedTotal, "runner_inventory_changed");
    for (const item of data.runners) {
      requireThat(
        Number.isSafeInteger(item?.id) && item.id > 0 && !seen.has(item.id),
        "runner_inventory_invalid",
      );
      seen.add(item.id);
      requireThat(item.id !== runnerId, "runner_absence_conflict");
    }
    requireThat(seen.size <= expectedTotal, "runner_inventory_invalid");
    if (seen.size === expectedTotal) return;
    requireThat(data.runners.length === 100, "runner_inventory_incomplete");
  }
  throw new Rejected("runner_inventory_incomplete");
}

async function writeOutput(handle, value) {
  await handle.writeFile(JSON.stringify(value, null, 2) + "\n");
  await handle.sync();
}

export async function execute(argv) {
  const args = options(argv);
  let output;
  try {
    if (args.output) output = await privateOutput(args.output);
    const token = await installationToken(args);
    if (args.mode === "issue") {
      const { data } = await request(
        token,
        "POST",
        `${ROOT}/generate-jitconfig`,
        "jit_issue",
        {
          name: args.name,
          runner_group_id: 1,
          labels: LABELS,
          work_folder: "_work",
        },
        [201],
      );
      // Retain the returned registration before subsequent identity checks so
      // a failed check cannot erase the only recoverable registration record.
      await writeOutput(output, {
        repository: REPOSITORY,
        runner: data?.runner,
        encoded_jit_config: data?.encoded_jit_config,
      });
      const runner = safeRunner(data?.runner);
      requireThat(
        runner.name === args.name &&
          typeof data.encoded_jit_config === "string" &&
          data.encoded_jit_config.length > 0 &&
          data.encoded_jit_config.length <= 65536,
        "jit_issue_response_invalid",
      );
      return { id: runner.id, name: runner.name, status: runner.status };
    }
    const runnerId = Number(args["runner-id"]);
    const runnerPath = `${ROOT}/${runnerId}`;
    const result = await request(token, "GET", runnerPath, "runner_get", undefined, [200, 404]);
    if (args.mode === "inspect") {
      if (result.status === 200) return safeRunner(result.data, runnerId);
      await absent(token, runnerId);
      return { id: runnerId, status: "absent", absent: true };
    }
    let busyObserved = null;
    if (result.status === 200) {
      const runner = safeRunner(result.data, runnerId);
      requireThat(runner.name === args["expected-name"], "runner_name_mismatch");
      requireThat(runner.busy === false, "runner_busy_refused");
      busyObserved = false;
      await request(token, "DELETE", runnerPath, "runner_remove", undefined, [204]);
      const confirmation = await request(
        token,
        "GET",
        runnerPath,
        "runner_remove_confirmation",
        undefined,
        [200, 404],
      );
      requireThat(confirmation.status === 404, "runner_removal_not_confirmed");
    }
    await absent(token, runnerId);
    const receipt = {
      generation_id: args["generation-id"],
      container_id: args["container-id"],
      repository: REPOSITORY,
      runner_id: runnerId,
      runner_name: args["expected-name"],
      busy: false,
      absent: true,
      observed_at: new Date().toISOString(),
      method: "github-rest-get-404",
      removal: busyObserved === null ? "already_absent" : "deleted_idle_runner",
      busy_observed: busyObserved,
    };
    await writeOutput(output, receipt);
    return { id: runnerId, name: args["expected-name"], status: "absent", absent: true };
  } finally {
    await output?.close();
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  execute(process.argv.slice(2)).then(
    (result) => process.stdout.write(JSON.stringify(result) + "\n"),
    (error) => {
      const reason = error instanceof Rejected ? error.message : "github_helper_failed";
      process.stderr.write(JSON.stringify({ result: "failed", reason }) + "\n");
      process.exitCode = 1;
    },
  );
}
