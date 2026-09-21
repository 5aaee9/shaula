// Real Terraform/provider plans against read-only local API fixtures, never apply.
// Usage: TERRAFORM=/path/to/terraform node scripts/template-backends/plan.mjs
import fs from 'node:fs/promises';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';

const repository = fileURLToPath(new URL('../../', import.meta.url));
const root = await fs.mkdtemp(path.join(os.tmpdir(), 'shaula-backend-plans-'));
const terraform = process.env.TERRAFORM || 'terraform';
for (const platform of ['docker', 'kubernetes']) {
  await fs.mkdir(path.join(root, platform, 'schemas'), { recursive: true });
  for (const file of ['main.tf', 'profile.yaml', '.terraform.lock.hcl',
    'schemas/bindings.schema.json', 'schemas/parameters.schema.json']) {
    await fs.copyFile(path.join(repository, 'templates', platform, file), path.join(root, platform, file));
  }
}
const images = (await fs.readFile(path.join(root, 'docker/profile.yaml'), 'utf8'))
  .split('\n').filter(line => line.includes('@sha256:')).map(line => line.trim().slice(2));
const imageId = `sha256:${'b'.repeat(64)}`;
const unexpected = [];
function provider(req, res) {
  const url = decodeURIComponent(req.url);
  if (!['GET', 'HEAD'].includes(req.method)) {
    unexpected.push(`${req.method} ${url}`);
    res.writeHead(405).end();
    return;
  }
  if (url.endsWith('/_ping')) {
    res.setHeader('API-Version', '1.41');
    res.end('OK');
    return;
  }
  res.setHeader('Content-Type', 'application/json');
  let body;
  if (url.endsWith('/version')) {
    body = { ApiVersion: '1.41', MinAPIVersion: '1.24', Version: '20.10.24', Os: 'linux', Arch: 'amd64' };
  } else if (url.endsWith('/images/json')) {
    body = images.map(image => ({ Id: imageId, RepoTags: [image, image.split('@')[0]], RepoDigests: [image] }));
  } else if (url.includes('/images/') && url.endsWith('/json')) {
    body = { Id: imageId, RepoTags: [], RepoDigests: [], Architecture: 'amd64', Os: 'linux', Size: 12345, Config: {} };
  } else if (url === '/api/v1/namespaces/runners') {
    body = { apiVersion: 'v1', kind: 'Namespace', metadata: { name: 'runners', uid: 'namespace-uid', resourceVersion: '1' }, spec: {}, status: { phase: 'Active' } };
  } else {
    unexpected.push(`${req.method} ${url}`);
    res.statusCode = 404;
    body = { message: 'unexpected fixture request', code: 404 };
  }
  res.end(JSON.stringify(body));
}
async function run(args) {
  const process = spawn(terraform, args, { stdio: ['ignore', 'pipe', 'pipe'] });
  let output = '', error = '';
  process.stdout.on('data', chunk => { output += chunk; });
  process.stderr.on('data', chunk => { error += chunk; });
  await new Promise((resolve, reject) => {
    process.on('error', reject);
    process.on('exit', code => code === 0 ? resolve() : reject(new Error(output + error)));
  });
  return output;
}
const docker = http.createServer(provider), kubernetes = http.createServer(provider);
try {
  const socket = path.join(root, 'provider.sock');
  await new Promise((resolve, reject) => { docker.once('error', reject); docker.listen(socket, resolve); });
  await new Promise((resolve, reject) => { kubernetes.once('error', reject); kubernetes.listen(0, '127.0.0.1', resolve); });
  const kubeconfig = path.join(root, 'kubeconfig');
  await fs.writeFile(kubeconfig, JSON.stringify({ apiVersion: 'v1', kind: 'Config',
    clusters: [{ name: 'test', cluster: { server: `http://127.0.0.1:${kubernetes.address().port}` } }],
    users: [{ name: 'test', user: { token: 'plan-only-fixture' } }],
    contexts: [{ name: 'test', context: { cluster: 'test', user: 'test' } }], 'current-context': 'test' }));
  for (const platform of ['docker', 'kubernetes']) {
    const directory = `-chdir=${path.join(root, platform)}`;
    await run([directory, 'init', '-backend=false', '-input=false', '-lockfile=readonly', '-no-color']);
    await run([directory, 'validate', '-no-color']);
    for (const backend of ['github', 'forgejo']) {
      const forgejo = backend === 'forgejo';
      const input = { contract_version: 1, generation: { fleet_key: 'fleet',
        id: '50a2dd2d-e48e-4b23-88de-b4bca9cf3b90', runner_name: 'runner', generation_name: 'generation',
        ...(forgejo ? {} : { scale_set_id: 1 }) }, jit_config: forgejo ? '' : 'frozen-jit',
      bindings_digest: 'commitment', parameters: {},
      bindings: platform === 'docker' ? { docker_host: `unix://${socket}` } : { namespace: 'runners', kubeconfig } };
      if (forgejo) {
        input.bindings.runner_backend = 'forgejo';
        input.forgejo = { instance_url: 'https://forgejo.test', uuid: 'runner-uuid', labels: ['linux:host'] };
      }
      await fs.writeFile(path.join(root, platform, 'fixture.tfvars.json'), JSON.stringify({ shaula: input }));
      await run([directory, 'plan', '-input=false', '-refresh=false', '-no-color', '-var-file=fixture.tfvars.json', '-out=fixture.plan']);
      const plan = await run([directory, 'show', '-json', 'fixture.plan']);
      await fs.writeFile(path.join(root, `${forgejo ? 'forgejo' : 'official'}-${platform}-plan.json`), plan);
    }
  }
  if (unexpected.length) throw new Error(`unexpected provider calls: ${unexpected.join(', ')}`);
  console.log('Validated both templates and saved four plans (fixture reads only; no apply).');
  console.log(`SHAULA_BOOTSTRAP_PLAN_ORACLES='${root}' cargo nextest run -p shaula-template -E 'test(captured_)'`);
  console.log(`Remove the temporary fixture directory after verification: ${root}`);
} finally {
  docker.closeAllConnections(); kubernetes.closeAllConnections();
  docker.close(); kubernetes.close();
}
