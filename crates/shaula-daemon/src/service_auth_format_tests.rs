//! Auth PUT format parsing tests (spec 0011 §3/§6).

#![allow(clippy::unwrap_used, clippy::panic)]

use super::AuthPutFormat;
use shaula_core::auth::AuthKind;
use shaula_core::github::GitHubTarget;
use shaula_core::registry::AuthProfilePut;
use shaula_core::secret::SecretString;

fn make_payload(
    kind: AuthKind,
    schema_version: Option<i64>,
    app_id: Option<&str>,
    installation_id: Option<i64>,
    pat: Option<&str>,
    allowlist: Option<Vec<GitHubTarget>>,
    target_policy: Option<Vec<shaula_core::auth_policy::TargetSelector>>,
) -> AuthProfilePut {
    AuthProfilePut {
        kind,
        app_id: app_id.map(str::to_string),
        installation_id,
        pat_identity: pat.map(str::to_string),
        secret: SecretString::new("pem-or-token"),
        allowlist,
        schema_version,
        target_policy,
    }
}

fn v2_policy() -> Vec<shaula_core::auth_policy::TargetSelector> {
    vec![
        shaula_core::auth_policy::TargetSelector::organization("Indexyz").unwrap(),
        shaula_core::auth_policy::TargetSelector::account_repositories(
            shaula_core::auth_policy::AccountKind::User,
            "5aaee9",
        )
        .unwrap(),
    ]
}

#[test]
fn v2_payload_parses_to_canonical_policy() {
    let payload = make_payload(
        AuthKind::GithubApp,
        Some(2),
        Some("4863460"),
        None,
        None,
        None,
        Some(v2_policy()),
    );
    let format = AuthPutFormat::parse(&payload).unwrap();
    match &format {
        AuthPutFormat::V2 {
            app_id,
            policy_json,
        } => {
            assert_eq!(app_id, "4863460");
            assert!(policy_json.contains("Indexyz"));
            assert!(policy_json.contains("5aaee9"));
            assert!(policy_json.contains("account_repositories"));
        }
        AuthPutFormat::Legacy { .. } => panic!("must parse as v2"),
    }
    // The canonical body is version-prefixed and never contains secrets.
    let body = format.canonical_body(&payload).unwrap();
    assert!(body.starts_with("2|4863460|"));
    assert!(!body.contains("pem-or-token"));
}

#[test]
fn legacy_payload_stays_legacy_and_rejects_policy_field() {
    let payload = make_payload(
        AuthKind::Pat,
        None,
        None,
        None,
        Some("octocat"),
        Some(vec![GitHubTarget::organization("o").unwrap()]),
        None,
    );
    assert!(matches!(
        AuthPutFormat::parse(&payload).unwrap(),
        AuthPutFormat::Legacy { .. }
    ));

    // A policy field without the explicit version is 422, never guessed
    // from the payload shape (spec 0011 §6).
    let guessing = make_payload(
        AuthKind::GithubApp,
        None,
        Some("4863460"),
        None,
        None,
        None,
        Some(v2_policy()),
    );
    assert!(AuthPutFormat::parse(&guessing).is_err());

    // schema_version 1 with a policy field is equally rejected.
    let one_with_policy = make_payload(
        AuthKind::GithubApp,
        Some(1),
        Some("4863460"),
        Some(34),
        None,
        Some(vec![GitHubTarget::organization("o").unwrap()]),
        Some(v2_policy()),
    );
    assert!(AuthPutFormat::parse(&one_with_policy).is_err());
}

#[test]
fn v2_mixed_members_and_bad_app_ids_are_rejected() {
    // Mixed legacy installation member.
    let mixed = make_payload(
        AuthKind::GithubApp,
        Some(2),
        Some("4863460"),
        Some(34),
        None,
        None,
        Some(v2_policy()),
    );
    assert!(AuthPutFormat::parse(&mixed).is_err());

    // Mixed allowlist member.
    let mixed = make_payload(
        AuthKind::GithubApp,
        Some(2),
        Some("4863460"),
        None,
        None,
        Some(vec![GitHubTarget::organization("o").unwrap()]),
        Some(v2_policy()),
    );
    assert!(AuthPutFormat::parse(&mixed).is_err());

    // PAT cannot be v2.
    let pat = make_payload(
        AuthKind::Pat,
        Some(2),
        None,
        None,
        Some("octocat"),
        None,
        Some(v2_policy()),
    );
    assert!(AuthPutFormat::parse(&pat).is_err());

    // Zero-padded or non-decimal App ids.
    for app_id in ["0", "0123", "Iv1.abc", "", "-5"] {
        let body = make_payload(
            AuthKind::GithubApp,
            Some(2),
            Some(app_id),
            None,
            None,
            None,
            Some(v2_policy()),
        );
        assert!(AuthPutFormat::parse(&body).is_err(), "{app_id}");
    }

    // Unknown schema version.
    let future = make_payload(
        AuthKind::GithubApp,
        Some(3),
        Some("4863460"),
        None,
        None,
        None,
        Some(v2_policy()),
    );
    assert!(AuthPutFormat::parse(&future).is_err());

    // The canonical legacy body covers every non-secret member.
    let legacy = make_payload(
        AuthKind::GithubApp,
        None,
        Some("12"),
        Some(34),
        None,
        Some(vec![GitHubTarget::organization("o").unwrap()]),
        None,
    );
    let format = AuthPutFormat::parse(&legacy).unwrap();
    let body = format.canonical_body(&legacy).unwrap();
    assert!(body.starts_with("github_app|12|34|"));
}
