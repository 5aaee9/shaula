---
status: accepted
---

# Retire v1 GitHub authentication and PAT

The v1 allowlist and single-installation model duplicated the v2 policy model
through publication, reads, validation and execution. A Fleet admission bug
parsed an empty legacy allowlist for a valid v2 revision. The operator has chosen
to remove this compatibility obligation and use v2 GitHub App authentication.

We retain one publication and authorization model: explicit TargetPolicy,
revision-scoped account bindings and exact execution contexts. We remove PAT,
v1 publication/replay, upgrade forms and reference-only runtime authorization.
Unsupported revisions fail closed. Strict version checks remain at trust
boundaries; future formats are not treated as v2 by numeric comparison.

Historical storage is preserved without conversion or credential decoding.
Dropping old columns would add destructive migration and rollback obligations
without improving the surviving runtime contract. Deployments must verify no
live or retained execution references require v1; inert history is permitted.

This supersedes the PAT/v1 support decisions in ADR-0007 and the compatibility
parts of ADR-0015/0016. OIDC management login is unaffected. The surviving
contract and rollout checks are owned by [spec 0018](../specs/0018-github-app-only-authentication.md).
