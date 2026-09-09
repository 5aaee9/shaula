use super::fixture::Fixture;
use shaula_core::github::ScaleSetIdentity;
use shaula_core::ports::{GitHubAccessPort, RemovalOutcome};
use std::sync::atomic::Ordering;

fn assert_numeric_scopes(f: &Fixture) {
    let bodies = f.script.token_bodies.read().unwrap();
    let metadata = serde_json::json!({"permissions": {"metadata": "read"}});
    let runner =
        serde_json::json!({"repository_ids": [700], "permissions": {"administration": "write"}});
    assert!(bodies.iter().any(|body| body == &metadata));
    assert!(bodies.iter().any(|body| body == &runner));
    assert!(
        bodies
            .iter()
            .all(|body| body == &metadata || body == &runner),
        "runner authority never uses names, an unscoped token or the discovery token"
    );
    assert!(f
        .script
        .metadata_bearers
        .read()
        .unwrap()
        .iter()
        .all(|bearer| bearer == "Bearer metadata-token"));
    let registrations = f.script.registration_bearers.read().unwrap();
    assert!(!registrations.is_empty());
    assert!(registrations
        .iter()
        .all(|bearer| bearer == "Bearer runner-token"));
}

#[tokio::test]
async fn repository_management_and_acquisition_use_numeric_runner_token_scope() {
    let f = Fixture::start().await;
    let client = f.client(true, true);
    let identity = ScaleSetIdentity {
        target: client.config.target.clone(),
        runner_group: "Default".into(),
        scale_set_name: "test".into(),
    };
    client.create_scale_set(&identity, 7, &[]).await.unwrap();
    client.update_scale_set_labels(9, &[]).await.unwrap();
    client.establish_session(9, "owner").await.unwrap();
    client.generate_jit(9, "runner").await.unwrap();
    client.acquire_jobs(9, &f.session(), &[1]).await.unwrap();
    assert_eq!(f.script.effects.load(Ordering::SeqCst), 5);
    assert_numeric_scopes(&f);
    f.advance(60_000);
    client.generate_jit(9, "runner").await.unwrap();
    assert_eq!(f.reads(), 2);
    assert_numeric_scopes(&f);
}

#[tokio::test]
async fn proof_only_runner_probe_discovers_identity_before_minting_runner_token() {
    let f = Fixture::start().await;
    f.client(true, false).probe_actions_access().await.unwrap();
    assert_numeric_scopes(&f);
    assert_eq!(f.script.effects.load(Ordering::SeqCst), 0);
    let bodies = f.script.token_bodies.read().unwrap();
    assert_eq!(bodies.len(), 2);
    assert_eq!(
        bodies[0],
        serde_json::json!({"permissions": {"metadata": "read"}})
    );
    assert_eq!(bodies[1]["repository_ids"], serde_json::json!([700]));
}

#[tokio::test]
async fn incomplete_metadata_never_mints_a_repository_runner_token() {
    let f = Fixture::start().await;
    f.script.repository.write().unwrap()["owner"]
        .as_object_mut()
        .unwrap()
        .remove("id");
    assert!(f.client(true, false).probe_actions_access().await.is_err());
    assert_eq!(
        &*f.script.token_bodies.read().unwrap(),
        &[serde_json::json!({"permissions": {"metadata": "read"}})]
    );
    assert!(f.script.registration_bearers.read().unwrap().is_empty());
}

#[tokio::test]
async fn retained_cleanup_requires_its_exact_numeric_identity() {
    let f = Fixture::start().await;
    let client = f.client(true, true);
    assert_eq!(
        client.remove_runner(77).await.unwrap(),
        RemovalOutcome::Removed
    );
    assert_eq!(f.script.cleanup_requests.load(Ordering::SeqCst), 1);
    assert_numeric_scopes(&f);
    f.script.repository.write().unwrap()["id"] = 999.into();
    f.advance(60_000);
    assert!(client.remove_runner(77).await.is_err());
    assert_eq!(
        f.script.cleanup_requests.load(Ordering::SeqCst),
        1,
        "a same-name replacement cannot receive a historical runner deletion"
    );
    let bodies = f.script.token_bodies.read().unwrap();
    assert!(!bodies
        .iter()
        .any(|body| body["repository_ids"] == serde_json::json!([999])));
}
