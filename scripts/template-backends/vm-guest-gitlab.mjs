// The real official gitlab-runner, verified by the bootstrap's own pinned SHA-256,
// polls a loopback GitLab API fixture. Bash sequencing, file modes and run-single's
// config.toml parsing execute for real; identity switching and devices are shims.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawn } from 'node:child_process';
import http from 'node:http';
import { copyFileSync, existsSync, mkdirSync, readFileSync, readlinkSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { assertGuestCaches, stageGuest } from './vm-guest.mjs';

const CONFIG = '/var/lib/shaula/gitlab-config.toml';
const NAME = '/var/lib/shaula/gitlab-runner-name';

// Answers every job request with 204 (no job), so run-single only polls.
export async function startGitlabApi() {
  const requests = [];
  const server = http.createServer((request, response) => {
    const chunks = [];
    request.on('data', chunk => chunks.push(chunk));
    request.on('end', () => {
      requests.push({ method: request.method, url: request.url, body: Buffer.concat(chunks).toString() });
      response.writeHead(request.method === 'POST' && request.url.endsWith('/api/v4/jobs/request') ? 204 : 404).end();
    });
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  return { requests, url: `http://127.0.0.1:${server.address().port}/gitlab`, close: () => server.close() };
}

function launch(path, env) {
  const child = spawn('bash', [path], { env, detached: true, stdio: ['ignore', 'pipe', 'pipe'] });
  let output = '';
  child.stdout.on('data', data => { output += data; });
  child.stderr.on('data', data => { output += data; });
  const exited = new Promise(resolve => child.on('exit', (status, signal) => resolve({ status, signal })));
  return { child, exited, output: () => output };
}

async function settle(run, api, before, label) {
  const timer = setTimeout(() => process.kill(-run.child.pid, 'SIGKILL'), 20000);
  const { status, signal } = await run.exited;
  clearTimeout(timer);
  assert.equal(signal, null, `${label}: bootstrap hung`);
  assert.notEqual(status, 0, `${label}: bootstrap must fail closed`);
  assert.equal(api.requests.length, before, `${label}: runner contacted GitLab`);
}

export async function exerciseGitlabGuest({ directory, files, script, platform, scenario, pythonPath, identity, token, binary, digest, api }) {
  const removed = { 'missing-config': [CONFIG], 'missing-name': [NAME] }[scenario] ?? [];
  const staged = stageGuest({ directory: join(directory, 'gitlab'), files, script, platform, scenario, pythonPath, download: binary, removed });
  const { root, path, env } = staged;
  if (scenario === 'preinstalled') {
    mkdirSync(root + '/opt/gitlab-runner', { recursive: true });
    copyFileSync(binary, root + '/opt/gitlab-runner/gitlab-runner');
  }
  const label = `${platform}/gitlab/${scenario}`;
  const before = api.requests.length;
  const run = launch(path, env);
  const leak = () => assert(!run.output().includes(token), `${label}: log leak`);
  if (!['success', 'preinstalled'].includes(scenario)) {
    await settle(run, api, before, label);
    leak();
    assert(existsSync(root + '/var/lib/shaula/started'), `${label}: seed not claimed`);
    return;
  }
  const deadline = Date.now() + 30000;
  const polled = () => api.requests.slice(before).find(request => request.url === '/gitlab/api/v4/jobs/request');
  while (!polled()) {
    assert(Date.now() < deadline, `${label}: run-single never polled: ${run.output()}`);
    assert.equal(run.child.exitCode, null, `${label}: exited early: ${run.output()}`);
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  const request = JSON.parse(polled().body);
  assert.equal(request.token, token);
  assert.equal(request.info.executor, 'shell');
  // bash -> runuser shim -> gitlab-runner all exec in place: one process.
  const pid = run.child.pid;
  const argv = readFileSync(`/proc/${pid}/cmdline`, 'utf8').split('\0').slice(0, -1);
  assert.deepEqual(argv, [root + '/opt/gitlab-runner/gitlab-runner', 'run-single', '--config', root + '/run/shaula-gitlab/config.toml',
    '--runner', identity.runner_name, '--max-builds', '1', '--wait-timeout', '600']);
  assert(!readFileSync(`/proc/${pid}/environ`, 'utf8').includes(token), `${label}: token in environment`);
  assert.equal(readlinkSync(`/proc/${pid}/cwd`), root + '/home/runner/gitlab-work');
  assert.equal(readFileSync(root + '/run/shaula-gitlab/config.toml', 'utf8'), files.get(CONFIG).decoded);
  assert.equal(statSync(root + '/run/shaula-gitlab/config.toml').mode & 0o777, 0o600);
  assert.equal(statSync(root + '/run/shaula-gitlab').mode & 0o777, 0o700);
  assert.equal(statSync(root + '/opt/gitlab-runner/gitlab-runner').mode & 0o777, 0o755);
  assert.equal(createHash('sha256').update(readFileSync(root + '/opt/gitlab-runner/gitlab-runner')).digest('hex'), digest);
  assert(!existsSync(root + CONFIG) && !existsSync(root + NAME), `${label}: handoff retained`);
  assert(!existsSync(root + '/opt/gitlab-runner/download'));
  assertGuestCaches(root, platform);
  process.kill(-pid, 'SIGKILL');
  await run.exited;
  leak();
  // The durable marker refuses a second start, even after the runner died.
  const after = api.requests.length;
  await settle(launch(path, env), api, after, `${label}/replay`);
  return { requests: api.requests.length - before, short_token_logged: run.output().includes(token.slice(5, 14)) };
}
