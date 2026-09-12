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
        app_id: None,
        target_policy: None,
        bindings: vec![],
        forgejo: None,
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
fn legacy_get_is_unsupported_without_old_authorization_fields() {
    let body = auth_profile_body(&view());
    assert_eq!(body["status"], "Unsupported");
    assert_eq!(body["active"]["state"], "Unsupported");
    assert_eq!(body["active"]["reason"], "UnsupportedAuthenticationFormat");
    for field in ["identity", "target_allowlist", "bindings", "app_id"] {
        assert!(body.get(field).is_none());
        assert!(body["active"].get(field).is_none());
    }
}

#[test]
fn historical_upgrade_does_not_make_old_active_revision_available() {
    let mut view = view();
    view.schema_version = 2;
    view.desired_revision = 2;
    view.app_id = Some("123".into());
    let mut candidate = legacy();
    candidate.revision = 2;
    candidate.schema_version = 2;
    candidate.state = "Rejected".into();
    candidate.reason = Some("AuthIdentityMismatch".into());
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
    assert_eq!(body["status"], "Unsupported");
    assert_eq!(body["active"]["state"], "Unsupported");
    assert!(body.get("identity").is_none());
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

#[test]
fn forgejo_reads_expose_only_provider_metadata_and_validation() {
    use shaula_core::{
        forgejo::{ForgejoScope, ForgejoTarget},
        registry::ForgejoAuthState,
    };
    let mut view = view();
    view.kind = Some(AuthKind::ForgejoToken);
    view.app_id = None;
    let target = ForgejoTarget {
        instance_url: "https://forgejo.test".into(),
        scope: ForgejoScope::User,
    };
    let active = view.active.as_mut().unwrap();
    active.forgejo = Some(ForgejoAuthState {
        target: target.clone(),
        validation: Some(shaula_core::ports::forgejo::ForgejoAuthProbe {
            server_version: "16.0.4".into(),
            principal_id: Some(42),
            target_id: None,
            checked_at_unix_ms: 10,
            valid_until_unix_ms: 60_010,
            runner_count: 1,
        }),
    });
    let mut candidate = active.clone();
    candidate.revision = 2;
    candidate.state = "Rejected".into();
    candidate.reason = Some("Unauthenticated".into());
    candidate.forgejo.as_mut().unwrap().validation = None;
    view.desired = Some(candidate);
    view.desired_revision = 2;
    view.live_fleets.push(shaula_core::registry::AuthLiveFleet {
        fleet_key: "pool".into(),
        phase: "Ready".into(),
        target: None,
        forgejo_target: Some(target),
    });
    let body = auth_profile_body(&view);
    assert_eq!(body["status"], "Active");
    assert_eq!(body["kind"], "forgejo_token");
    assert_eq!(body["active"]["state"], "Active");
    assert_eq!(body["active"]["forgejo"]["validation"]["principal_id"], 42);
    assert_eq!(body["desired"]["state"], "Rejected");
    assert_eq!(body["desired"]["reason"], "Unauthenticated");
    assert!(body["desired"]["forgejo"]["validation"].is_null());
    assert_eq!(body["liveFleets"][0]["kind"], "forgejo");
    assert_eq!(
        body["liveFleets"][0]["target"]["instance_url"],
        "https://forgejo.test"
    );
    for field in [
        "token",
        "credential_bytes",
        "app_id",
        "bindings",
        "target_policy",
    ] {
        assert!(body.get(field).is_none());
        assert!(body["active"].get(field).is_none());
        assert!(body["active"]["forgejo"].get(field).is_none());
    }
}
