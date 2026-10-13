# GitLab VM bootstrap: local guest evidence (2026-10-14)

## What this proves

For all four VM templates (Proxmox, AWS, TencentCloud, AliCloud), the
`runner_backend: gitlab` user-data rendered by real Terraform 1.9.8 boots
the official gitlab-runner 19.4.1 binary in a relocated guest fixture.
The binary is verified by the bootstrap's own pinned SHA-256. `run-single`
then authenticates to a loopback GitLab API fixture with exactly the
rendered single-registration token.

Per platform, [`results.json`](results.json) records:

- the encoded user-data size (16 KiB bound);
- the bootstrap SHA-256;
- the runner digest;
- the guest cases, including whether the short token appeared in runner output.

## Checks

- **Render:** `gitlab-config.toml` and `gitlab-runner-name` are root `0600`
  handoffs. The token is absent from the bootstrap, the unit, provider
  credentials and other backends' renders. The unit has `Restart=no` and
  `NoNewPrivileges=true`, with no `[Install]`.
- **Success:** the real binary sends `POST /gitlab/api/v4/jobs/request` with the
  token and `executor: shell`. `/proc/<pid>` shows:
  - argv exactly `run-single --config …/run/shaula-gitlab/config.toml --runner <name> --max-builds 1 --wait-timeout 600`;
  - no token in argv or environment;
  - cwd `…/home/runner/gitlab-work`.

  The config is `0600` in a `0700` directory. The handoffs and download are
  removed, and the guest caches (plus the Proxmox seed) are root-only. A replay
  is refused by the durable marker.
- **Proxmox `preinstalled`:** an exact pre-installed binary is reused. The curl
  shim fails if it is called.
- **Fail closed:** pre-start failure, checksum mismatch, missing config, missing
  runner name and the Proxmox seed failures all exit non-zero. None of them
  contacts the API. The full token never appears in bootstrap or runner output.
  The runner's own log prints the 9-character short token, as spec 0043 §5
  records.

## Not proven

These are test substitutes:

- identity switching (`runuser`), `useradd` and `apt-get`;
- block devices and udev;
- cloud-init and systemd.

The fixture API never hands out a job. These still need real-environment
acceptance:

- a job executed on a VM;
- VM/ISO/disk cleanup;
- the real cloud metadata exposure.

## Reproduce

```sh
nix develop -c env PYTHON=/path/to/python3 node scripts/template-backends/vm.mjs
```

Optionally set `GITLAB_RUNNER_BINARY` to a pre-downloaded official binary; it is
still checked against the pinned digest.
