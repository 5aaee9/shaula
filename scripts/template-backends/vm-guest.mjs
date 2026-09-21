// Test-only OS shims. File permissions, checksum verification, Bash sequencing and
// the Python argv builder execute for real; identity switching and block devices do not.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, rmSync, statSync, symlinkSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';

export function exerciseGuest({ directory, files, script, platform, scenario, pythonPath, identity, token }) {
  const root = join(directory, scenario);
  const helpers = join(root, 'bin');
  mkdirSync(helpers, { recursive: true, mode: 0o700 });
  const record = join(root, 'record.json');
  for (const [path, file] of files) {
    const destination = root + path;
    mkdirSync(dirname(destination), { recursive: true });
    writeFileSync(destination, file.decoded, { mode: Number.parseInt(file.permissions, 8) });
  }
  for (const path of ['/var/lib/cloud', '/run/cloud-init', '/dev']) mkdirSync(root + path, { recursive: true });
  writeFileSync(join(root, 'dev/seed'), 'fixture ISO');
  if (scenario === 'missing-token') rmSync(root + '/var/lib/shaula/forgejo-token');
  if (scenario === 'pre-start-failure') writeFileSync(root + '/var/lib/shaula/pre-start', 'exit 27\n');
  const binary = `#!${pythonPath}
import json, os, pathlib, stat, sys, urllib.parse
path = urllib.parse.urlparse(sys.argv[sys.argv.index('--token-url') + 1]).path
token = pathlib.Path(path).read_text()
record = dict(argv=sys.argv[1:], cwd=os.getcwd(), token_in_argv=any(token in arg for arg in sys.argv),
              token_in_env=any(token in value for value in os.environ.values()),
              token_mode=stat.S_IMODE(os.stat(path).st_mode), dir_mode=stat.S_IMODE(os.stat(os.path.dirname(path)).st_mode))
pathlib.Path(os.environ['FIXTURE_RECORD']).write_text(json.dumps(record))
`;
  const fixtureBinary = join(root, 'fixture-binary');
  writeFileSync(fixtureBinary, binary);
  const helper = (name, body, interpreter = '/bin/sh') => {
    const path = join(helpers, name);
    writeFileSync(path, `#!${interpreter}\n${body}\n`, { mode: 0o700 });
  };
  symlinkSync(pythonPath, join(helpers, 'python3'));
  helper('apt-get', 'exit 0');
  helper('id', '[ "$1" != "-u" ] || printf "1001\\n"');
  helper('useradd', 'exit 1'); // id fixture already exists; never create a real user.
  helper('curl', `while [ "$#" -gt 0 ]; do
  if [ "$1" = "--output" ]; then cp "$FIXTURE_BINARY" "$2"; target="$2"; break; fi
  shift
done
[ -n "$target" ] || exit 1
if [ "$FIXTURE_SCENARIO" = checksum-failure ]; then printf corrupt >> "$target"; fi`);
  const install = spawnSync('which', ['install'], { encoding: 'utf8' }).stdout.trim();
  assert(install.startsWith('/'));
  helper('install', `import os, sys
args = []
source = iter(sys.argv[1:])
for item in source:
    if item in ('-o', '-g'):
        assert next(source) == 'runner'
    else:
        if item.startswith('/'):
            assert item.startswith(os.environ['FIXTURE_ROOT'] + '/')
        args.append(item)
os.execv(${JSON.stringify(install)}, [${JSON.stringify(install)}] + args)`, pythonPath);
  helper('runuser', '[ "$1" = "-u" ] && [ "$2" = runner ] && [ "$3" = -- ] || exit 1\nshift 3\nexec "$@"');
  helper('udevadm', 'exit 0');
  helper('blkid', 'printf "%s/dev/seed\\n" "$FIXTURE_ROOT"');
  helper('chown', '[ "$FIXTURE_SCENARIO" != seed-protect-failure ]');
  helper('findmnt', `case "$FIXTURE_SCENARIO" in
  seed-lookup-failure) exit 2;;
  unmount-failure) printf '%s/mounted\\n' "$FIXTURE_ROOT";;
  *) exit 1;;
esac`);
  helper('umount', '[ "$FIXTURE_SCENARIO" != unmount-failure ]');
  for (const prefix of ['/var/lib/shaula', '/var/lib/cloud', '/run/cloud-init', '/opt/forgejo-runner', '/run/shaula-forgejo', '/home/runner/forgejo-work']) {
    script = script.replaceAll(prefix, root + prefix);
  }
  // Fixture block device is a regular file, without mknod/root privileges.
  if (platform === 'proxmox') script = script.replace('test -b "$seed"', 'test -f "$seed"');
  // Verify the fake downloaded binary through the actual sha256sum check.
  script = script.replace('29dae21e93f0eab5cdf3564008d44603c74770b41a4f4f1aceed172c774bc376', createHash('sha256').update(binary).digest('hex'));
  const bash = spawnSync('which', ['bash'], { encoding: 'utf8' }).stdout.trim();
  assert(bash.startsWith('/'));
  script = script.replaceAll('/bin/bash', bash); // Ubuntu guest path on Nix hosts.
  const path = join(root, 'bootstrap.sh');
  writeFileSync(path, script);
  const env = { ...process.env, PATH: `${helpers}:${process.env.PATH}`, FIXTURE_ROOT: root, FIXTURE_RECORD: record, FIXTURE_BINARY: fixtureBinary, FIXTURE_SCENARIO: scenario };
  const result = spawnSync('bash', [path], { env, encoding: 'utf8', timeout: 20000 });
  if (result.error) throw result.error;
  assert(!`${result.stdout}${result.stderr}`.includes(token), `${platform}/${scenario}: log leak`);
  assert(existsSync(root + '/var/lib/shaula/started'), `${platform}/${scenario}: seed not claimed`);
  assert.equal(existsSync(record), scenario === 'success', `${platform}/${scenario}: ${result.stderr}`);
  if (scenario !== 'success') {
    assert.notEqual(result.status, 0, `${platform}/${scenario}: bootstrap must fail closed`);
    return;
  }
  assert.equal(result.status, 0, `${platform}: ${result.stderr}`);
  const observed = JSON.parse(readFileSync(record));
  assert.deepEqual(observed.argv, ['one-job', '--url', identity.instance_url, '--uuid', identity.uuid, '--token-url', `file://${root}/run/shaula-forgejo/token`, ...identity.labels.flatMap(label => ['--label', label]), '--wait']);
  assert.equal(observed.cwd, root + '/home/runner/forgejo-work');
  assert.equal(observed.token_in_argv, false);
  assert.equal(observed.token_in_env, false);
  assert.equal(observed.token_mode, 0o600);
  assert.equal(observed.dir_mode, 0o700);
  assert.equal(statSync(root + '/var/lib/cloud').mode & 0o777, 0o700);
  assert.equal(statSync(root + '/run/cloud-init').mode & 0o777, 0o700);
  if (platform === 'proxmox') assert.equal(statSync(root + '/dev/seed').mode & 0o777, 0o600);
  assert(!existsSync(root + '/var/lib/shaula/forgejo-token'));
  const timestamp = statSync(record).mtimeMs;
  const replay = spawnSync('bash', [path], { env, encoding: 'utf8', timeout: 20000 });
  assert.notEqual(replay.status, 0, `${platform}: replay started`);
  assert.equal(statSync(record).mtimeMs, timestamp);
}
