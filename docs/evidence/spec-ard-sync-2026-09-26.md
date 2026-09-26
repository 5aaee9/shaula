# Code / SPEC / ARD synchronization review — 2026-09-26

## Baseline and scope

Reviewed remote `main` at `7e548d93dc81b56aecc24c6544099cc1307711e0`
(`fix(diagnostics): complete DX-30 acceptance`, 2026-09-26 13:03:37 UTC).
The connected GitHub API supplied commit-pinned files after local Git network
access failed. This is a source/documentation review, not a runtime acceptance
run or a claim that every requirement in every specification is implemented.

The review followed the documentation index into workspace/CLI ownership,
GitHub versus management authentication, template follow/replacement, inline
and shared pools, and the latest diagnostics acceptance boundary. Changes are
limited to Markdown. [Implementation status](../IMPLEMENTATION_STATUS.md)
remains the sole progress record; this file records why this particular
synchronization patch was made.

## Confirmed drift and corrections

| Area | Source evidence at the baseline | Documentation correction |
| --- | --- | --- |
| Workspace and CLI | [Workspace manifest](../../Cargo.toml), [binary dispatch](../../crates/shaula/src/main.rs), [client dependencies](../../crates/shaula-client/Cargo.toml), [transport dependencies](../../crates/shaula-api-types/Cargo.toml) | Spec 0007 and ARD-0010 now include `shaula-api-types`, `shaula-client`, `shaula-forgejo` and remote HTTP-client commands. Internal `job` remains a target requirement, not an implemented CLI assertion. |
| GitHub credentials | [Credential enum and chain](../../crates/shaula-scaleset/src/auth.rs) only admit GitHub App; [spec 0018](../specs/0018-github-app-only-authentication.md) retires the legacy formats. | Remove obsolete successful GitHub PAT/oracle acceptance requirements from spec 0007 and ARD-0010. Keep legacy-format rejection and distinguish [Shaula personal tokens](../specs/0039-user-access-tokens-and-cli.md) from GitHub PATs. |
| Follow-latest PUT | [Admission material resolution](../../crates/shaula-daemon/src/service_fleet_materials.rs) retains a single-template pin only when both the reference and inputs are unchanged; changed inputs resolve current Active. | Spec 0023 describes the explicit inputs-replacement case and its existing zero-occupancy fence. ARD-0028 also points to ARD-0029's follow-only amendment instead of advertising a current pinned mode. |
| Inline versus shared pools | [Routing and cap scope](../../crates/shaula-store/src/registry_impl/lifecycle_pool_admit.rs) distinguishes historical inline members from a frozen shared-pool revision; [spec 0037](../specs/0037-shared-template-pool-resource.md) owns new shared-pool admissions. | Spec 0029 and ARD-0036 identify the historical inline contract and link the replacement contract. The index no longer calls accepted weighted selection “Proposed” or describes it as weighted acquisition. |
| Pool HTTP contract | [Router](../../crates/shaula-http/src/router/mod.rs), [handlers](../../crates/shaula-http/src/router/pool_routes.rs) and [strict body types](../../crates/shaula-core/src/template_pool.rs) use `/api/v1/template-pools/{key}` and deserialize the direct spec, with the key from the URL. | Spec 0037 and ARD-0037 fix the route prefix; the spec fixes conditions/scopes and invalid top-level `key` example; the replacement is parseable JSON rather than a GET envelope. Migration references point to section 8. |
| Pool follow barriers | [Pool cascade](../../crates/shaula-daemon/src/service_follow_pool.rs) can mint a new pool revision while referencing Fleets are occupied; [Fleet cascade](../../crates/shaula-daemon/src/service_follow_cascade.rs) applies the Fleet's zero-occupancy boundary. | Spec 0037 and ARD-0037 no longer say both cascade stages wait for Fleet occupancy to reach zero; ARD-0037 also names the same-pool-revision cap scope. |
| Diagnostics acceptance index | [DX-30 evidence](0041-dx30-hyperv-2026-09-26/README.md) records all four scenarios passing for the selected Forgejo/Docker tuple. | The index replaces the blanket “real-platform acceptance pending” label with links to the exact evidence and remaining boundaries. It does not claim GitHub, all platforms, ordinary busy-safe drain or a separate registration-DELETE failure passed. |

## Deliberately not changed

- [Spec 0010](../specs/0010-lifecycle-worker-and-http-state-backend.md) and
  ARD-0014 remain the worker/state target. [Current serve wiring](../../crates/shaula/src/serve.rs)
  still constructs `TemplateRuntime::new`; the absence of the complete worker
  integration is not a reason to delete its safety requirements or mark it done.
- Spec 0041 section 2 explicitly describes its historical inspected commit.
  Those historical observations are not relabeled as current defects after
  the implementation landed.
- An implemented backend or checked-in template does not automatically change
  a Draft/proposed design decision into an accepted one. Existing acceptance
  statuses and unverified platform boundaries are preserved.
- No lifecycle, credential, schema, migration, policy, CI workflow or test code
  changes are included. Existing evidence and historical test counts are not
  replaced with results from this review.

## Validation boundary

The edited originals were reconstructed from commit-pinned API reads and their
Git blob SHA-1 values checked against GitHub before modification. Local checks
cover `git diff --check`, Markdown fence balance, the new JSON example's syntax
and field shape, and newly introduced relative links against the inspected
repository files/tree. These are documentation checks, not Rust execution.

Cargo, rustfmt, Clippy and cargo-nextest were unavailable in this environment;
`cargo fmt`, `cargo clippy` and the requested workspace nextest command were
**not run**. No Terraform, real GitHub/Forgejo lifecycle, browser suite or
provider fault-injection test was rerun. The DX-30 claim above is attributed
to the repository's existing receipts, not to this documentation pass.
