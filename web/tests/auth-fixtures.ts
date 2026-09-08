/**
 * The REAL legacy wire shape (F1): identity is the composed string, the
 * allowlist carries config URLs, and there is no app_id member.
 */
export const LEGACY_APP_PROFILE = {
  key: "legacy-app",
  incarnation: "auth-inc",
  desiredRevision: 1,
  activeRevision: 1,
  status: "Active",
  kind: "github_app",
  identity: "app/Iv23legacy/installation/34",
  credential_present: true,
  target_allowlist: ["https://github.com/acme", "https://github.com/acme/build-tools"],
};

export const V2_PROFILE = {
  key: "shared-github",
  incarnation: "auth-inc",
  desiredRevision: 2,
  activeRevision: 1,
  status: "Active",
  kind: "github_app",
  credential_present: true,
  schema_version: 2,
  app_id: "4863460",
  active: {
    revision: 1,
    state: "Active",
    reason: null,
    schema_version: 2,
    app_id: "4863460",
    target_policy: [
      { kind: "organization", owner: "Indexyz" },
      {
        kind: "account_repositories",
        account_kind: "user",
        owner: "5aaee9",
      },
    ],
    bindings: [
      {
        account_id: 100,
        account_kind: "organization",
        login: "Indexyz",
        installation_id: 11,
        repository_selection: "all",
        validated_at_ms: 4,
        health: "Validated",
        reason: null,
        checked_at_ms: 4,
        valid_until_ms: 60_004,
        affected_fleets: [],
      },
    ],
  },
  desired: {
    revision: 2,
    state: "Validating",
    reason: null,
    schema_version: 2,
    app_id: "4863460",
    target_policy: [
      { kind: "organization", owner: "Indexyz" },
      {
        kind: "account_repositories",
        account_kind: "user",
        owner: "5aaee9",
      },
    ],
  },
};
