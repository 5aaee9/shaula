# Non-waiting one-job: real protocol experiment (2026-09-21)

## Decision supported by this experiment

**Nominal execution/cleanup works, but removing `--wait` is not a safe general
idle-reclamation implementation for the tested official runner.** Do not switch
production bootstrap or infer deletion authority from process exit plus remote
`idle`. No production acquisition proxy was added.

This follows the operator's request to first validate demand-driven creation,
non-waiting `one-job`, and cleanup after exit. It evaluates the alternative
suggested by [discussion 66](https://codeberg.org/forgejo/discussions/issues/66#issuecomment-10593860)
and the [KEDA example](https://keda.sh/docs/2.19/scalers/forgejo/); it does not adopt
KEDA's shared registration or privileged DinD configuration.

## Real environment and scope

- Local Linux amd64, rootless Podman; official Forgejo **16.0.4** and official
  runner **13.1.0**. Image IDs and repository digests are retained in
  [`results.json`](results.json).
- A fresh disposable Forgejo, private repositories, one distinct label and one
  ephemeral registration per case. The generated workflows only echo, sleep for
  three seconds and exit with a specified status, using the `:host` backend.
- The controller-like harness waits for a waiting job before creating its runner
  (except the intentional empty-queue case). It observes execution, waits for
  natural process exit, and records registration cleanup separately from local
  container/volume cleanup.
- **Not a Shaula Rust Pool/Terraform integration test**, Kubernetes conformance,
  production idle drain, or full A1–A8 acceptance. Actual resource removal here
  uses the local engine CLI, not Terraform Destroy.

## Results

All six experiment assertions passed. A passing negative case reproduces an
unsafe condition; it does **not** mean the proposed production policy passed.

| Case | Runner exit | Forgejo observation after exit | Reclamation evidence |
| --- | --- | --- | --- |
| No matching task | `2` | Registration remains `idle`; no jobs | Harness explicitly deletes the registration, then removes the stopped container. It knows the entire queue was empty; production cannot infer that from exit `2`. |
| Task succeeds | `0` | Workflow `success`; exact registration ID absent | Running container preserved; after natural exit, container and anonymous volume removed. |
| Task fails | `0` | Workflow `failure`; exact registration ID absent | Same cleanup. Runner exit `0` does **not** mean workflow success. |
| Fetch returns HTTP 503 before forwarding | `2` | Registration `idle`; job still `waiting`, no task ID | No reclamation authorized; evidence retained until isolated fixture teardown. |
| Server assigns task but response is lost | `2` | Registration still **`idle`**, but job **`running` with a task ID** | No reclamation authorized. This disproves `exited + idle = safe to delete`. |
| Same response loss, with `--wait` as a control | `0` | A second FetchTask occurs; workflow succeeds; registration disappears | Runner recovers the assignment and finishes before resource removal. |

The injector forwards the actual FetchTask to Forgejo, consumes its complete
HTTP 200 response, then drops the client connection. It does not invent a fake
`running` observation. The control allows the next request through. The retained
report contains request counts/status only, no request keys, tokens or task
payloads. The injector is disposable test equipment, **not** a Shaula protocol
proxy or acquisition fence.

## Interpretation

The official single-task poller returns `ErrNoTaskReceived` when no task is
returned and `wait=false`, even when FetchTask requested reuse of its request key
following a transport error. The CLI maps this error to exit code `2`. With
`--wait`, the loop retries instead. See the pinned
[poller](https://code.forgejo.org/forgejo/runner/src/commit/667c8d975b9255e7bf32164e012146f68f0022c6/internal/app/poll/single.go)
and [CLI](https://code.forgejo.org/forgejo/runner/src/commit/667c8d975b9255e7bf32164e012146f68f0022c6/internal/app/cmd/cmd.go).

Two consequences matter for Shaula:

1. A completed ephemeral registration can disappear and its resource can then
   be destroyed after independent resource/identity checks. Both successful and
   failed workflows need this cleanup; exit-code-based workflow classification
   would be wrong.
2. A never-executed runner can exit while holding an unresolved server-side
   assignment. Its inventory status can still be idle. Blanket DELETE/Destroy
   would discard that ambiguity, not resolve it. Re-running a new process is
   also not proven recovery: the original in-memory request key is lost.

No production code, bootstrap flags, admission rules or deletion safeguards
were changed. Runtime observation and full Terraform cleanup integration remain
unverified. The broader active-drain blocker is documented in
[`forgejo-drain.md`](../../forgejo-drain.md).

## Reproduce

```sh
node --test scripts/forgejo-one-job/fault.test.mjs
CONTAINER_ENGINE=podman node scripts/forgejo-one-job/verify.mjs > results.json
```

See the [harness README](../../../scripts/forgejo-one-job/README.md) for setup and
isolation constraints. The three fault-injector unit tests pass. The real
experiment confirms all fixture containers and anonymous volumes are gone at
teardown; the downloaded images remain cached. Tokens and raw runner/RPC logs
are not retained in this evidence directory.

Additional repository checks for this increment:

- `cargo fmt --all`: passed.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `cargo nextest run --manifest-path Cargo.toml --workspace test`: 671 passed,
  179 skipped (the required `test` name filter excludes some integration tests).
- `cargo nextest run --manifest-path Cargo.toml --workspace`: 848 passed,
  2 skipped.
- JavaScript formatting check and all three fault-injector unit tests: passed.

These local checks remain separate from the real protocol experiment and do
not establish full platform or Terraform conformance.
