//! The publication boundary supports only explicit GitHub App v2 policies.

#![allow(clippy::unwrap_used)]

use super::AuthPutFormat;
use shaula_core::auth::AuthKind;
use shaula_core::auth_policy::{AccountKind, TargetSelector};
use shaula_core::registry::AuthProfilePut;
use shaula_core::secret::SecretString;

fn payload() -> AuthProfilePut {
    AuthProfilePut {
        kind: AuthKind::GithubApp,
        schema_version: Some(2),
        app_id: Some("4863460".into()),
        secret: SecretString::new("fixture-private-key"),
        target_policy: Some(vec![
            TargetSelector::organization("Indexyz").unwrap(),
            TargetSelector::account_repositories(AccountKind::User, "5aaee9").unwrap(),
        ]),
    }
}

#[test]
fn policy_body_is_canonical_and_excludes_credentials() {
    let format = AuthPutFormat::parse(&payload()).unwrap();
    assert_eq!(format.app_id, "4863460");
    assert!(format.policy_json.contains("account_repositories"));
    assert!(format.canonical_body().starts_with("2|4863460|"));
    assert!(!format.canonical_body().contains("fixture-private-key"));
}

#[test]
fn old_unknown_or_missing_versions_and_pat_are_rejected() {
    for version in [None, Some(1), Some(3)] {
        let mut request = payload();
        request.schema_version = version;
        assert!(AuthPutFormat::parse(&request).is_err());
    }
    let mut request = payload();
    request.kind = AuthKind::Pat;
    assert!(AuthPutFormat::parse(&request).is_err());
}

#[test]
fn invalid_identity_or_missing_policy_is_rejected() {
    for app_id in ["0", "0123", "Iv1.abc", "", "-5"] {
        let mut request = payload();
        request.app_id = Some(app_id.into());
        assert!(AuthPutFormat::parse(&request).is_err(), "{app_id}");
    }
    let mut request = payload();
    request.target_policy = None;
    assert!(AuthPutFormat::parse(&request).is_err());
}
