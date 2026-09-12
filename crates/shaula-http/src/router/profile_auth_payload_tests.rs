#![allow(clippy::unwrap_used)]

use super::{build_auth_payload, AuthProfilePutDto};
use serde_json::json;

fn request() -> serde_json::Value {
    json!({
        "kind": "github_app", "schema_version": 2, "app_id": "123",
        "private_key": "fixture-key", "target_policy": [{"kind":"organization","owner":"acme"}],
    })
}

#[test]
fn supported_publication_is_accepted() {
    let payload = build_auth_payload(serde_json::from_value(request()).unwrap()).unwrap();
    assert_eq!(payload.schema_version, Some(2));
    assert_eq!(payload.app_id.as_deref(), Some("123"));
}

#[test]
fn legacy_members_are_unknown_even_when_null_or_empty() {
    for field in [
        "target_allowlist",
        "installation_id",
        "pat_principal",
        "token",
    ] {
        for value in [json!(null), json!([]), json!("")] {
            let mut body = request();
            body[field] = value;
            assert!(
                serde_json::from_value::<AuthProfilePutDto>(body).is_err(),
                "{field}"
            );
        }
    }
}

#[test]
fn forgejo_token_requires_and_preserves_scoped_target() {
    let body = json!({
        "kind": "forgejo_token",
        "instance_url": "https://forgejo.example.test",
        "scope": {"kind": "repository", "owner": "acme", "name": "repo"},
        "token": "forgejo-secret"
    });
    let payload = build_auth_payload(serde_json::from_value(body).unwrap()).unwrap();
    assert_eq!(payload.kind, shaula_core::auth::AuthKind::ForgejoToken);
    assert_eq!(payload.schema_version, Some(1));
    assert_eq!(payload.secret.expose(), "forgejo-secret");
    assert!(payload.forgejo_target.is_some());
}

#[test]
fn old_missing_and_unknown_formats_are_rejected() {
    for version in [json!(null), json!(1), json!(3)] {
        let mut body = request();
        body["schema_version"] = version;
        assert!(build_auth_payload(serde_json::from_value(body).unwrap()).is_err());
    }
    let mut body = request();
    body.as_object_mut().unwrap().remove("schema_version");
    assert!(build_auth_payload(serde_json::from_value(body).unwrap()).is_err());
    for kind in ["pat", "github-app"] {
        let mut body = request();
        body["kind"] = json!(kind);
        assert!(build_auth_payload(serde_json::from_value(body).unwrap()).is_err());
    }
}
