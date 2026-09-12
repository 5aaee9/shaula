# Forgejo A1–A8 acceptance evidence (2026-09-12)

This record separates local implementation evidence from real Forgejo
acceptance. The acceptance contract requires both; local tests do not promote
themselves to real-platform evidence.

| Item | Local evidence | Real-platform status |
| --- | --- | --- |
| A1 closure | `shaula-forgejo` adapter tests and the Pool lifecycle tests cover registration material, inventory readiness and ephemeral cleanup paths. | **Partial real smoke.** [GitHub Actions run 34694263009](https://github.com/5aaee9/shaula/actions/runs/34694263009) passed Forgejo 16.0.4 + runner 13.1.0 registration, matching job completion and ephemeral disappearance. It still does not compose Shaula's Rust Pool driver or record an explicit idle inventory snapshot. |
| A2 labels | Label normalization and ownership subset checks are covered in `shaula-forgejo` and `shaula-daemon` tests. | **Not accepted.** A real run with matching and non-matching `runs-on` jobs is still required. |
| A3 demand snapshot | Replacement-based waiting/running arithmetic and stale-demand preservation pass in the Pool and adapter tests. | **Not accepted.** No real poll sequence was run. |
| A4 busy-safe drain | Active runners remain Busy; idle inventory alone does not authorize deletion; positive local runtime evidence is required. | **Blocked.** The pinned Forgejo protocol has no acquisition fence or busy-conditional delete. See `docs/forgejo-drain.md`. |
| A5 uncertain registration | HTTP tests cover unreadable/ambiguous responses; the classifier covers None/ExactlyOne/Quarantined and Pool tests cover quarantine/no repost. | **Not accepted.** Response-loss injection against a live instance is still required. |
| A6 association level | Generic Jobs projection tests preserve `Unverified`/`Ambiguous` safeguards; Forgejo currently does not emit a `Verified` association. | **Gap.** Forgejo job observations are not yet projected into the Jobs view, so a real Forgejo Jobs/API assertion is still required. |
| A7 leak scan | Secret redaction, bootstrap boundary and tfvars identity tests pass; no token is exposed in debug material or rendered bindings. | **Not accepted.** A live resource/argv/log/setup-info scan is still required. |
| A8 no regression | Full workspace nextest passes: 803 passed, 2 skipped. Provider wiring is isolated from GitHub paths. | **Local only.** Real GitHub regression evidence remains separate. |

Local commands used for this record:

```text
cargo nextest run -p shaula-forgejo -p shaula-store -p shaula-daemon -p shaula-core -p shaula-http forgejo --no-fail-fast --status-level fail --final-status-level fail
cargo nextest run --manifest-path Cargo.toml --workspace --no-fail-fast --status-level fail --final-status-level fail
```

The first command passed 28 Forgejo-filtered tests. The second passed 802
tests with 2 skipped. Neither command is real-platform evidence.
