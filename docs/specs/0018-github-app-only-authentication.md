# GitHub App authentication without the legacy format

Status: accepted. This specification supersedes the v1 authentication, PAT,
legacy publication/replay and upgrade contracts in specs 0005, 0011 and 0012.
The versioned v2 policy, bindings and exact execution context contracts remain.

## Supported contract

GitHub authentication publications require `kind: github_app`,
`schema_version: 2`, a positive decimal `app_id`, a private key and an explicit
`target_policy`. Missing, older and unknown schema versions are rejected.
PAT, fixed installation ID and `target_allowlist` publications are rejected
before persistence or replay. The UI offers only the GitHub App policy form.

Fleet admission evaluates only the active revision's TargetPolicy. Validation,
credential construction, handoff, session installation and retained execution
require a v2 GitHub App revision and its exact context. Missing policy or context
must never fall back to an allowlist, another revision or reference-only
authorization. Unsupported revisions cannot be selected for new references.

## Historical data and rollout

Existing revision rows, credential bytes, audit facts, operation evidence and
revision numbers are preserved. Old database columns are historical storage,
not a supported input or authorization representation. There is no automatic
conversion of an old installation, exact allowlist or PAT into a policy or
numeric identity binding. Historical revisions can be identified as unsupported
without decoding or exposing their old credential metadata.

An unsupported active profile must be replaced by an explicitly published v2
profile. A pending v2 publication does not make its old active revision usable.
Old execution references fail closed and retain ownership/cleanup evidence;
they require explicit recovery with the previous release before rollout.
Deployment preflight must inspect active/desired profile revisions and retained
Fleet/session/operation references. Inert old history alone must not prevent
startup. This release does not drop columns or delete historical rows, so code
rollback does not require restoring or rewriting the database.

## Acceptance

- Valid v2 publications, rotations, dynamic personal repositories and multiple
  organizations continue to work with the existing authorization gates.
- Missing/v1/unknown versions, PAT and every old publication member are refused;
  a matching historical idempotency entry cannot bypass this check.
- Fleet admission rejects an old active revision even if its old allowlist
  would have allowed the target; malformed/missing policy never grants access.
- Runtime and persistence cannot acknowledge reference-only handoffs or create
  sessions/effects with old authentication. Rejected work preserves evidence.
- UI has no PAT, installation ID, exact-allowlist or legacy-upgrade form, and
  cannot select unsupported historical profiles.
- Existing database history remains readable and unmodified. Pure reads do not
  migrate, activate, expand policy or contact GitHub.
