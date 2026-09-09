# External Docker Template smoke test

[文档索引](README.md) · [项目首页](../README.md)

Run the commands below from the repository root.

This Linux-only Python standard-library harness executes the Docker Template
through the exact Terraform subprocess supplied by the operator. Its host-side Docker copy/start verifies the official-image bootstrap ordering.
The production Runtime has its own fixed bootstrap under spec 0020; this external
harness does not exercise the production archive/projection integration.
It uses a protected **local** state workspace and does not implement or prove the
spec 0010 `exec` Driver, independent `shaula job` worker, or database HTTP backend.

The generated sanitized report always has `full_conformance_passed: false`.
It is evidence for the scenarios actually executed, not a passing activation
attestation. Automatic static activation follows spec 0017; this report does not establish
full runtime conformance.

## Run

Use a dedicated disposable directory owned by the invoking administrator, mode
0700, containing an exact materialized copy of the reviewed Template and its real
provider lock. Pre-pull the pinned official GitHub Runner image into the intended Docker Engine.
Keep the caller's original input JSON outside the workspace, mode 0600, with sole
top-level key `shaula`; JIT is `shaula.jit_config`, and the Docker binding is
`shaula.bindings.docker_host`. Never place secrets directly on the command line.
The JIT `.runner` document must contain the actual `agentId` and `agentName`;
the name must equal `shaula.generation.runner_name`. These identities are frozen
before apply. Later phases require the original socket and Docker daemon ID.

```sh
python3 scripts/docker-conformance/run.py prepare \
  --workspace /protected/smoke/template \
  --terraform /nix/store/EXACT-terraform/bin/terraform \
  --docker /run/current-system/sw/bin/docker \
  --socket unix:///var/run/docker.sock \
  --vars /protected/smoke/input.tfvars.json \
  --image ghcr.io/actions/actions-runner:2.337.0@sha256:e5496277be5d09bc968b3d64911b74e219ac4a3f2edce956a3ecf9271bea1ef4 \
  --report /protected/smoke/report.json
```

`prepare` freezes the input in `.docker-conformance`, runs readonly locked init
and validation, admits one saved create-only plan with empty prior state, then
applies it once. It checks `start=false`, `must_run=false`, `rm=false`, the
official image and native Listener/JIT input in the real plan. After apply it
verifies the exact stopped container, copies a host-generated `.setup_info` and
reads back the same bytes before recording `start_possible` and starting it.
The fixed group is `Shaula bootstrap smoke test`; its text explicitly says this
marker does not prove production operation-log delivery. No raw Terraform output
is put into the marker. The harness
rejects a repeated prepare even if the first apply was uncertain. State, input,
plan, and journal remain protected in the workspace. Each subprocess retains raw
stdout and stderr in exclusive 0600 files under `.docker-conformance/commands`,
named by bounded phase and random command ID. These diagnostics may contain JIT,
credentials or state and must stay protected; they are never printed or included
in the sanitized report. Both streams are drained concurrently and capped at
16 MiB each on disk, with at most 16 MiB of stdout retained in memory. Exceeding
the limit or timeout kills the command process group, retains the bounded prefix,
and reports an uncertain outcome; this is not proof that an escaped descendant
was fenced. Inspect the protected logs directly for an apply failure diagnosis.

Run `inspect` with the same workspace, executables, socket, and report arguments
after `Runner.Listener` starts and **before** its one job completes. It checks
the exact image/container identity, ownership labels, non-root user, restart,
auto-removal, namespaces, capabilities, devices, mounts, declarative metadata,
the official Listener command and process argv. The sole permitted secret env
input is `ACTIONS_RUNNER_INPUT_JITCONFIG`; Docker configuration retains it and
remains protected. No container exec, shim or staged JIT file is used.

Independently verify the registered runner identity and online status with
GitHub, dispatch a bounded manual job selecting its unique label, and retain
the job result and workflow-environment checks. Container running status alone
is not runner readiness. Do not print job environment or credentials to test
redaction: assert absence and report only bounded check results.

After the job, use the authoritative GitHub API to establish the exact runner
is not busy and is safely removed. Write an owner-only receipt from that actual
observation, not from elapsed time, container exit, or a desired outcome:

```json
{
  "generation_id": "EXACT_GENERATION_ID",
  "container_id": "EXACT_CONTAINER_ID",
  "repository": "5aaee9/shaula",
  "runner_id": 123,
  "runner_name": "EXACT_JIT_RUNNER_NAME",
  "busy": false,
  "absent": true,
  "method": "github-rest-get-404",
  "observed_at": "2026-09-08T12:00:00Z"
}
```

The receipt is an operator trust boundary, not an independently authenticated
GitHub proof. A 404 counts only after successful authenticated inventory/read
of the intended repository and runner; an authorization failure cannot prove
absence. A complete paginated inventory may instead use
`method: github-rest-inventory`. The harness requires an observation no more
than five minutes old. Run `destroy` with the common arguments and
`--removal-evidence /protected/smoke/removal.json`. It requires the original
state's container, admits the exact saved delete-only plan, applies it, and
proves `terraform state list` is empty and that exact container is absent.
The receipt runner ID/name must match the frozen JIT identity. The saved delete
plan must carry that exact original container ID in its `change.before`, as well
as the correct Terraform type and address.

Terraform 1.9.8 may leave the deleted resource's condition check `unknown`
because its [destroy planning node](https://github.com/hashicorp/terraform/blob/v1.9.8/internal/terraform/node_resource_plan_destroy.go)
plans deletion and checks `prevent_destroy` without evaluating ordinary resource
preconditions. The external harness accepts this unevaluated status only for
the exact, fully identified `docker_container.runner` resource present in both
the bound state and a delete-only plan, with no check problems or nested check
results. Create-side unknown checks, failures, and unowned checks still reject.
This smoke-only compatibility rule does not change or prove the production Rust
worker composition or full conformance. The Rust Destroy admission has a
separate regression fix for this same observed condition; Create remains strict.

No phase invokes native Docker create, delete, adoption, or state removal. An
uncertain Create or Destroy requires manual investigation and process fencing;
this harness does not implement recovery or automatically retry an apply.
Retain the protected workspace until uncertainty is resolved. Even after proven
Destroy it is retained for operator-controlled disposal; there is no unconditional
cleanup that can erase evidence. Do not publish raw state, plans, inputs, or the
private journal.

## Evidence limits and checks

The report records the tested source-file commitment, actual engine version and
binary digest, dependency-lock digest and provider checksum commitments. The
source commitment includes paths, modes and bytes of every file, including
extensionless helpers. Source symlink files/directories and non-regular files are
rejected. Only the generated root directories `.terraform` and
`.docker-conformance`, and the root files `terraform.tfstate`,
`terraform.tfstate.backup`, `errored.tfstate`, `.terraform.tfstate.lock.info` and
`crash.log` are excluded. Put the report outside the Template or inside
`.docker-conformance`. This harness supports the bundled, self-contained single
root module only: external modules, helper executables/files outside the
workspace, environment-based template inputs, or other unpinned dependencies
are unsupported and not covered by the source commitment. Providers and the
runner image have their separate pins and commitments.

The provider commitment matches Rust: sort the full checksum list, append a newline
to every entry, and SHA-256 those bytes. It does not invent an artifact archive
digest or an opaque server-issued binding commitment. Attestation assembly must
separately bind the exact archive and admitted binding revision and retain all
required full-suite evidence.

The smoke marker does not verify production apply-log projection or Jobs archive
retention. Real GitHub job success, ordinary workflow environment/context, management
HTTP/audit/log/telemetry redaction, fault injection, lifecycle worker composition,
HTTP state semantics, and capability fencing require separate tests. The harness
does not prove `/proc` isolation or memory zeroization. Same-identity IaC children
share ambient Docker host authority, and Terraform state retains bootstrap
material until its protected retention lifecycle completes.

Run the meaningful offline refusal checks with:

```sh
python3 -m unittest discover -s scripts/docker-conformance -p 'test_*.py'
```

These checks exercise unsafe container shapes, create/replacement refusal,
destroy state coverage, the single approved native JIT env exception, host bootstrap refusal, and checksum compatibility.
They do not substitute for a real Docker/GitHub run.

## Local GitHub App helper

`github.mjs` uses Node.js 20+ built-ins and only `https://api.github.com`. Run it
on the machine holding the existing App private key; never copy that key or an
installation token to the Docker host. Every command takes `--app-id`,
`--installation-id` (the personal account installation), `--private-key` (an
absolute file path), and optionally `--repository 5aaee9/shaula`; other
repositories are refused. It verifies the installation belongs to `5aaee9` and
requests a token restricted to this repository's runner administration.

```text
node scripts/docker-conformance/github.mjs issue <common flags> --name RUNNER_NAME --output ABSOLUTE_PRIVATE_JIT_JSON
node scripts/docker-conformance/github.mjs inspect <common flags> --runner-id RUNNER_ID
node scripts/docker-conformance/github.mjs remove <common flags> --runner-id RUNNER_ID --expected-name RUNNER_NAME --generation-id GENERATION_ID --container-id CONTAINER_ID --output ABSOLUTE_PRIVATE_RECEIPT_JSON
```

`issue` registers one JIT runner in group 1 with labels `self-hosted`, `linux`,
`x64`, and `shaula-codex-docker`; its private JSON contains `runner` and
`encoded_jit_config`. Keep its exact ID/name for later commands. Stdout only
contains bounded runner identity/status; credentials and JIT are never printed.
`inspect` returns the runner's ID/name/status/busy, or a verified absence result.

`remove` refuses a busy runner or a name mismatch, deletes only the exact ID,
then requires GET 404 and a complete authenticated repository runner inventory
without that ID. An already absent ephemeral runner follows the same inventory
check; supply the expected name from the saved issue result, never a guess.
Its receipt is bound to the supplied Generation/container and exact runner.
For already absent runners, `busy_observed` is null: `busy: false` describes
the absent registration, not a fabricated observation of an idle runner.

Output files are created exclusively and are never overwritten. On Unix, use
an owner-only directory (0700); the helper creates files as 0600 and requires an
owner-only private key. On Windows, first restrict and verify the output parent
directory's NTFS ACL for the invoking user: Node's Unix mode bits do not enforce
Windows ACLs. Keep that directory private throughout execution. A failed request
may leave an empty reserved output or a returned registration for investigation;
there are no automatic issue/delete retries. Inspect GitHub before deciding how
to recover an uncertain result. Dispatch the smoke workflow separately using
the operator's GitHub user authorization.
