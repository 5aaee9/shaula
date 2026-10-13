#!/usr/bin/env node
// Provider-free Terraform evaluation of the exact bundled user_data expressions.
// Then exercise Bash + Python launchers with guest paths relocated to private fixtures.
// No cloud calls, package installs, user creation or mounts. GitHub/Forgejo Runners are
// fakes; GitLab runs the real official binary (downloaded once, or GITLAB_RUNNER_BINARY)
// against a loopback API fixture that never hands out a job.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { chmodSync, copyFileSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { exerciseGuest } from './vm-guest.mjs';
import { exerciseGitlabGuest, startGitlabApi } from './vm-guest-gitlab.mjs';

const repository = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const terraform = process.env.TERRAFORM || 'terraform';
const python = process.env.PYTHON || 'python3';
const root = mkdtempSync(join(tmpdir(), 'shaula-vm-backends-'));
chmodSync(root, 0o700);
const token = 'vm-canary-"quote"-\\-$()-`data`-秘密';
const identity = { instance_url: 'https://forgejo.test/subpath', uuid: "runner'uuid", labels: ['linux:host', 'quoted"label:host'] };
const api = await startGitlabApi();
// TOML basic strings refuse quotes/backslashes, so the canaries carry shell syntax instead.
const gitlabToken = 'glrt-vm-canary-$()-`data`-秘密';
const gitlab = { instance_url: api.url, runner_id: 42, runner_name: "shaula-gl $(never) `run` 'name'" };
// Mirrors GitlabBootstrapMaterial::config_toml, the only accepted rendering.
const configToml = `concurrent = 1\ncheck_interval = 3\n\n[[runners]]\n  name = "${gitlab.runner_name}"\n  url = "${gitlab.instance_url}"\n  id = ${gitlab.runner_id}\n  token = "${gitlabToken}"\n  executor = "shell"\n`;
const bindings = {
  proxmox: { proxmox_host: 'https://pve.test', proxmox_token: 'fixture@pve!test=provider-canary' },
  aws: { aws_region: 'us-east-1', aws_access_key_id: 'AKIAFIXTUREONLY0000', aws_secret_access_key: 'provider-canary' },
  tencentcloud: { tencentcloud_region: 'ap-guangzhou', tencentcloud_secret_id: 'fixture', tencentcloud_secret_key: 'provider-canary', tencentcloud_availability_zone: 'ap-guangzhou-6', tencentcloud_image_id: 'img-fixture', tencentcloud_vpc_id: 'vpc-fixture', tencentcloud_subnet_id: 'subnet-fixture', tencentcloud_security_group_ids: ['sg-fixture'] },
  alicloud: { alicloud_region: 'cn-hangzhou', alicloud_access_key_id: 'fixture', alicloud_access_key_secret: 'provider-canary', alicloud_image_id: 'ubuntu_22_04_x64', alicloud_vswitch_id: 'vsw-fixture', alicloud_security_group_ids: ['sg-fixture'] },
};

export function run(command, args, options = {}) {
  const result = spawnSync(command, args, { encoding: 'utf8', timeout: 60000, maxBuffer: 4 * 1024 * 1024, ...options });
  if (result.error) throw result.error;
  assert.equal(result.status, 0, `${command} failed: ${result.stderr}`);
  return result.stdout;
}

// The official binary for this host, checked against the digest pinned in the bootstrap.
let official;
function officialGitlabRunner(script) {
  const arch = { x64: 'amd64', arm64: 'arm64' }[process.arch];
  assert(arch, `unsupported host architecture ${process.arch}`);
  const version = script.match(/^RUNNER_VERSION=(\S+)$/m)?.[1];
  const digest = script.match(new RegExp(`RUNNER_ARCH=${arch}\\n\\s+RUNNER_SHA256=([0-9a-f]{64})`))?.[1];
  assert(version && digest, 'GitLab Runner pin missing');
  if (!official) {
    let path = process.env.GITLAB_RUNNER_BINARY;
    if (!path) {
      path = join(root, `gitlab-runner-linux-${arch}`);
      run('curl', ['--fail', '--silent', '--show-error', '--location', '--retry', '3', '--output', path,
        `https://gitlab-runner-downloads.s3.amazonaws.com/v${version}/binaries/gitlab-runner-linux-${arch}`], { timeout: 300000 });
    }
    official = { path, digest };
    assert.equal(createHash('sha256').update(readFileSync(path)).digest('hex'), digest, 'official binary differs from the pin');
  }
  assert.equal(digest, official.digest, 'platforms pin different GitLab Runner binaries');
  return official;
}

run(python, ['--version']);
const pythonPath = run(python, ['-c', 'import sys; print(sys.executable)']).trim();
const evidence = [];
try {
  for (const platform of Object.keys(bindings)) {
    const source = join(repository, 'templates', platform);
    const main = readFileSync(join(source, 'main.tf'), 'utf8');
    const expression = main.match(/^  user_data = ((?:base64encode\()?templatefile\([\s\S]*?^  \}\)\)?)/m)?.[1];
    assert(expression, `${platform}: user_data expression missing`);
    const suffix = main.match(/^ {2}backend_suffix\s+= .*$/m)?.[0];
    assert(suffix, `${platform}: backend_suffix missing`);
    const variable = main.slice(main.indexOf('variable "shaula"'));
    assert(variable.startsWith('variable "shaula"'));
    const directory = join(root, platform);
    mkdirSync(directory);
    for (const file of readdirSync(source).filter(name => name.endsWith('.tftpl'))) copyFileSync(join(source, file), join(directory, file));
    // Keep the actual variable declaration/defaults/validation and user_data expression;
    // omit provider/resource blocks so evaluation cannot contact or provision a cloud.
    const decoded = ['tencentcloud', 'alicloud'].includes(platform) ? 'base64decode(local.user_data)' : 'local.user_data';
    writeFileSync(join(directory, 'main.tf'), `${variable}\nlocals {\n${suffix}\n  user_data = ${expression}\n  decoded = ${decoded}\n}\n`);
    run(terraform, ['init', '-backend=false', '-input=false', '-no-color'], { cwd: directory });
    for (const backend of ['github', 'forgejo', 'gitlab']) {
      const forgejo = backend === 'forgejo';
      const isGitlab = backend === 'gitlab';
      const input = { contract_version: 1, generation: { id: 'fixture', generation_name: 'fixture', fleet_key: 'fixture' }, jit_config: backend === 'github' ? 'github-jit-canary' : '', bindings_digest: 'digest', bindings: { ...bindings[platform] }, parameters: {} };
      if (forgejo) Object.assign(input, { forgejo: identity, forgejo_vm: { token } });
      if (isGitlab) Object.assign(input, { gitlab, gitlab_vm: { config_toml: configToml } });
      // Omit GitHub selection to verify the legacy default.
      if (backend !== 'github') input.bindings.runner_backend = backend;
      input.bindings[`${platform}_cloud_init_cmd`] = ': # literal ${not_a_template} %{not_a_directive} $() `data`';
      writeFileSync(join(directory, 'shaula.tfvars.json'), JSON.stringify({ shaula: input }), { mode: 0o600 });
      const output = run(terraform, ['console', '-no-color', '-var-file=shaula.tfvars.json'], {
        cwd: directory, input: 'nonsensitive(jsonencode({ cloud = yamldecode(local.decoded), encoded_bytes = length(base64encode(local.decoded)) }))\n',
      });
      const rendered = JSON.parse(JSON.parse(output.trim()));
      assert(rendered.encoded_bytes <= 16384, `${platform}: base64 size exceeded`);
      const files = new Map(rendered.cloud.write_files.map(file => [file.path, { ...file, decoded: file.encoding === 'b64' ? Buffer.from(file.content, 'base64').toString() : file.content }]));
      const all = [...files.values()].map(file => file.decoded).join('\n');
      assert(!all.includes('provider-canary'));
      assert(!all.includes('AKIAFIXTUREONLY0000'));
      assert.deepEqual(rendered.cloud.runcmd, [['systemctl', 'daemon-reload'], ['systemctl', 'start', '--no-block', 'shaula-runner.service']]);
      assert(files.get('/var/lib/shaula/pre-start').decoded.includes('${not_a_template}'));
      const service = files.get('/etc/systemd/system/shaula-runner.service').decoded;
      assert(service.includes('Restart=no') && service.includes('ConditionPathExists=!/var/lib/shaula/started'));
      assert(!service.includes('[Install]'));
      const script = files.get('/usr/local/libexec/shaula-runner').decoded;
      const scriptPath = join(directory, `bootstrap-${backend}.sh`);
      writeFileSync(scriptPath, script);
      run('bash', ['-n', scriptPath]);
      if (backend !== 'github') {
        assert(!files.has('/var/lib/shaula/jit-config'));
        assert(!all.includes('github-jit-canary'));
        assert(service.includes('NoNewPrivileges=true'));
      }
      if (forgejo) {
        assert.equal(files.get('/var/lib/shaula/forgejo-token').decoded, token);
        assert.deepEqual(JSON.parse(files.get('/var/lib/shaula/forgejo-identity').decoded), identity);
        for (const path of ['/var/lib/shaula/forgejo-token', '/var/lib/shaula/forgejo-identity']) {
          assert.equal(files.get(path).permissions, '0600');
          assert.equal(files.get(path).owner, 'root:root');
        }
        assert(!all.includes(gitlabToken));
        assert(script.includes('RUNNER_VERSION=13.1.0'));
        assert(script.includes('RUNNER_SHA256=29dae21e93f0eab5cdf3564008d44603c74770b41a4f4f1aceed172c774bc376'));
        assert(!script.includes(token));
        const cases = ['success', 'pre-start-failure', 'checksum-failure', 'missing-token'];
        if (platform === 'proxmox') cases.push('seed-protect-failure', 'seed-lookup-failure', 'unmount-failure');
        for (const scenario of cases) exerciseGuest({ directory, files, script, platform, scenario, pythonPath, identity, token });
        evidence.push({ platform, backend, encoded_bytes: rendered.encoded_bytes, guest_cases: cases, bootstrap_sha256: createHash('sha256').update(script).digest('hex') });
      } else if (isGitlab) {
        assert(!files.has('/var/lib/shaula/forgejo-token'));
        assert(!all.includes(token));
        assert.equal(files.get('/var/lib/shaula/gitlab-config.toml').decoded, configToml);
        assert.equal(files.get('/var/lib/shaula/gitlab-runner-name').decoded, gitlab.runner_name);
        for (const path of ['/var/lib/shaula/gitlab-config.toml', '/var/lib/shaula/gitlab-runner-name']) {
          assert.equal(files.get(path).permissions, '0600');
          assert.equal(files.get(path).owner, 'root:root');
        }
        assert(!script.includes(gitlabToken) && !service.includes(gitlabToken));
        assert(script.includes('RUNNER_VERSION=19.4.1'));
        assert(script.includes('--max-builds 1 --wait-timeout 600'));
        const { path: binary, digest } = officialGitlabRunner(script);
        const cases = ['success', 'pre-start-failure', 'checksum-failure', 'missing-config', 'missing-name'];
        if (platform === 'proxmox') cases.push('preinstalled', 'seed-protect-failure', 'seed-lookup-failure', 'unmount-failure');
        const observed = {};
        for (const scenario of cases) {
          observed[scenario] = await exerciseGitlabGuest({ directory, files, script, platform, scenario, pythonPath, identity: gitlab, token: gitlabToken, binary, digest, api });
        }
        evidence.push({ platform, backend, encoded_bytes: rendered.encoded_bytes, guest_cases: cases, runner_sha256: digest, polls: observed.success, bootstrap_sha256: createHash('sha256').update(script).digest('hex') });
      } else {
        assert(!files.has('/var/lib/shaula/forgejo-token') && !files.has('/var/lib/shaula/gitlab-config.toml'));
        assert.equal(files.get('/var/lib/shaula/jit-config').decoded, 'github-jit-canary');
        assert(!all.includes(token) && !all.includes(gitlabToken));
        evidence.push({ platform, backend, encoded_bytes: rendered.encoded_bytes });
      }
    }
  }
} finally {
  api.close();
}
writeFileSync(join(root, 'evidence.json'), JSON.stringify(evidence, null, 2));
console.log(JSON.stringify({ fixture_root: root, result: 'pass', checks: evidence }, null, 2));
