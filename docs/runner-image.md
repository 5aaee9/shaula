# Docker Runner bootstrap image

[文档索引](README.md) · [项目首页](../README.md)

Run the commands below from the repository root.

This image supplies `/usr/local/bin/bootstrap-shim` for the Docker Terraform
Profile. It starts the existing `Runner.Listener run` directly, consumes
`/shaula/jit_config`, deletes that file before execution, and has no restart loop.
The file must contain canonical base64 of the Runner JSON dictionary, containing
`.runner`, `.credentials`, and `.credentials_rsaparams` with base64 JSON objects.
Unknown configuration filenames, malformed/empty input, symlinks, non-regular
files and encoded input larger than 64 KiB fail closed with a fixed diagnostic.

The current recipe also installs `/usr/local/lib/shaula/setup_info.py`. A v2
Template may stage `/shaula/setup_info.json`; after successful JIT validation the
shim uses that optional descriptor to fetch approved apply output over HTTPS,
merge `.setup_info`, then exec the Listener. A missing/disabled descriptor adds
no network request. Setup delivery failures degrade independently of JIT errors.
The existing bundled Profile still pins its previous v1 image; build a new image
and use [the v2 source generator](setup-info-templates.md) to enable delivery.

The official base image must include `/usr/bin/python3` and the Listener; the
build checks both and makes no package updates. Python uses isolated mode and
only its standard library. The image runs as `runner`; `/shaula` belongs to that
user with mode `0700`. The pinned Docker provider 3.0.2 uploads non-executable
files with mode `0644`, so `runner` can read the root-owned JIT upload and remove
it from its own private directory. The provider has no upload `permissions`
attribute; leave `executable` false. This mode comes from the provider's
[fixed upload implementation](https://github.com/kreuzwerker/terraform-provider-docker/blob/v3.0.2/internal/provider/resource_docker_container_funcs.go#L428).
The shim sets umask `077` before the Listener writes its credential files. It
does not mount a socket or require container privilege.

The reviewed base is Runner 2.337.0, pinned to the official multi-platform
index digest below. Its Linux amd64 child manifest is
`sha256:5036480998280bb21e32ade9fe1b02b493861ac314b62ba1aea320b94f56ec97`.
Build this tuple explicitly for Linux amd64:

```sh
runner_base_image='ghcr.io/actions/actions-runner:2.337.0@sha256:e5496277be5d09bc968b3d64911b74e219ac4a3f2edce956a3ecf9271bea1ef4'
shim_image_tag='shaula-runner:2.337.0-bootstrap-v2'
docker build --platform linux/amd64 \
  --build-arg RUNNER_BASE_IMAGE="$runner_base_image" \
  -t "$shim_image_tag" templates/docker/image
```

`runner_base_image` must have the form
`ghcr.io/actions/actions-runner:<version>@sha256:<64 lowercase hex>` (the tag is
optional). A tag without a digest is rejected. Publish or import the built image
and record its final immutable digest in the Profile; the base digest is not the
final shim image digest.

Run the complete, credential-free tests on Linux with Python 3.9 or later:

```sh
python3 -m unittest discover -s templates/docker/image -p 'test_*.py' -v
```

The tests execute a fake Listener, verify the staged file is absent before it
starts, verify argv and the single JIT input environment variable, reject a
second consumption, propagate the Listener exit status, and exercise malformed,
missing, oversized, symlink, FIFO and unlink-failure cases. The fake Listener
explicitly simulates environment removal; this does not test the real Listener's
behavior. Non-Linux runs cover only wire validation and skip file/exec tests.

The upstream mechanism was checked against Runner **v2.337.0**:

- [CommandSettings.cs](https://github.com/actions/runner/blob/v2.337.0/src/Runner.Listener/CommandSettings.cs#L101)
  captures `ACTIONS_RUNNER_INPUT_*` in its private argument dictionary, masks
  declared secrets and removes these environment entries before running a job;
  `GetJitConfig()` reads the captured value.
- [Runner.cs](https://github.com/actions/runner/blob/v2.337.0/src/Runner.Listener/Runner.cs#L217)
  decodes the JIT dictionary and writes its configuration files before running.

Recheck those mechanisms for the exact base version chosen for release. This
unit suite is not a Docker Profile conformance attestation. The real image,
Terraform/provider tuple, GitHub job, redaction and cleanup/recovery evidence
remain required by spec 0006. The same Runner execution domain can inspect
process memory; removing the ordinary environment entry does not establish
process isolation or memory zeroization.
