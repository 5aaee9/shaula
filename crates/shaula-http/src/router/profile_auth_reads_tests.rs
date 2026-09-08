#![allow(clippy::unwrap_used)]
use super::auth_profile_body;
use shaula_core::auth::AuthKind;
use shaula_core::auth_context::{AccountBinding, RepositorySelection};
use shaula_core::auth_policy::{AccountKind, TargetPolicy};
use shaula_core::registry::{AuthBindingHealth, AuthProfileView, AuthRevisionState};

fn legacy() -> AuthRevisionState {
    AuthRevisionState {
        revision: 1,
        state: "Active".into(),
        reason: None,
        binding_health: vec![],
        schema_version: 1,
        identity: Some("app/Iv23legacy/installation/34".into()),
        target_allowlist: vec![
            "https://github.com/acme".into(),
            "https://github.com/acme/repo".into(),
        ],
        app_id: None,
        target_policy: None,
        bindings: vec![],
    }
}

fn view() -> AuthProfileView {
    AuthProfileView {
        key: "legacy-app".into(),
        incarnation: "auth-inc".into(),
        desired_revision: 1,
        active_revision: Some(1),
        status: "Active".into(),
        kind: Some(AuthKind::GithubApp),
        credential_present: true,
        schema_version: 1,
        app_id: Some("Iv23legacy".into()),
        active: Some(legacy()),
        desired: None,
        live_fleets: vec![],
    }
}

#[test]
fn pure_legacy_get_preserves_exact_baseline_shape_used_by_upgrade_form() {
    assert_eq!(
        auth_profile_body(&view()),
        serde_json::json!({
            "key": "legacy-app", "incarnation": "auth-inc", "desiredRevision": 1,
            "activeRevision": 1, "status": "Active", "kind": "github_app", "credential_present": true,
            "identity": "app/Iv23legacy/installation/34",
            "target_allowlist": ["https://github.com/acme", "https://github.com/acme/repo"],
        })
    );
}

#[test]
fn failed_upgrade_retains_legacy_active_and_attributes_rejection_to_candidate() {
    let mut view = view();
    view.schema_version = 2;
    view.desired_revision = 2;
    view.app_id = Some("123".into());
    let mut candidate = legacy();
    candidate.revision = 2;
    candidate.schema_version = 2;
    candidate.state = "Rejected".into();
    candidate.reason = Some("AuthIdentityMismatch".into());
    candidate.identity = None;
    candidate.app_id = Some("123".into());
    candidate.target_policy = Some(
        serde_json::from_str::<TargetPolicy>(
            r#"{"selectors":[{"kind":"organization","owner":"acme"}]}"#,
        )
        .unwrap(),
    );
    view.desired = Some(candidate);
    let body = auth_profile_body(&view);
    assert_eq!(body["active"]["schema_version"], 1);
    assert_eq!(body["identity"], "app/Iv23legacy/installation/34");
    assert_eq!(body["desired"]["state"], "Rejected");
    assert_eq!(body["desired"]["reason"], "AuthIdentityMismatch");
    assert_eq!(body["desired"]["bindings"], serde_json::json!([]));
}

#[test]
fn v2_binding_health_needs_the_exact_account_and_installation_evidence() {
    let mut view = view();
    view.schema_version = 2;
    let active = view.active.as_mut().unwrap();
    active.schema_version = 2;
    active.bindings.push(AccountBinding {
        account_id: 1,
        account_kind: AccountKind::User,
        login: "person".into(),
        installation_id: 34,
        repository_selection: RepositorySelection::All,
        validated_at_ms: 100,
    });
    active.binding_health.push(AuthBindingHealth {
        account_id: 1,
        installation_id: 35,
        state: "Validated".into(),
        reason: None,
        checked_at_ms: Some(100),
        valid_until_ms: Some(60_100),
        affected_fleets: vec![],
    });
    assert_eq!(
        auth_profile_body(&view)["active"]["bindings"][0]["health"],
        "Unknown"
    );
    view.active.as_mut().unwrap().binding_health[0].installation_id = 34;
    let body = auth_profile_body(&view);
    assert_eq!(body["active"]["bindings"][0]["health"], "Validated");
    assert_eq!(body["active"]["bindings"][0]["valid_until_ms"], 60_100);
    assert!(body.get("identity").is_none());
    assert!(body.get("target_allowlist").is_none());
}
