//! Corrupt evidence and legacy-session replacement regressions.
#![allow(clippy::unwrap_used)] // Fixed fixture assertions.
use super::auth_execution::ready;
use super::auth_v2::{binding, policy_json, promotion, seed_v2_candidate, PROFILE};
use sea_orm::{ConnectionTrait, DbBackend, Statement};

async fn execute(store: &crate::Store, sql: &str) {
    store.connection().execute_unprepared(sql).await.unwrap();
}

#[tokio::test]
async fn observed_json_cannot_disagree_with_its_durable_reference() {
    let (store, captured, context) = ready().await;
    let json = serde_json::to_string(&context).unwrap();
    store
        .handoff_acknowledge("fleet", PROFILE, 1, Some(&json), &captured)
        .await
        .unwrap();
    execute(
        &store,
        "UPDATE fleet_auth_contexts SET observed_profile_key='other' WHERE fleet_key='fleet'",
    )
    .await;
    assert!(store
        .handoff_acknowledge("fleet", PROFILE, 1, Some(&json), &captured)
        .await
        .is_err());
    assert_eq!(
        store
            .fleet_auth_context_get("fleet")
            .await
            .unwrap()
            .unwrap()
            .observed_profile_key
            .as_deref(),
        Some("other")
    );
    execute(&store, "UPDATE fleet_auth_contexts SET observed_profile_key='shared-github',observed_context_json='{broken' WHERE fleet_key='fleet'").await;
    assert!(store
        .handoff_acknowledge("fleet", PROFILE, 1, Some(&json), &captured)
        .await
        .is_err());
    assert_eq!(
        store
            .auth_execution_context_get("fleet", PROFILE, 1)
            .await
            .unwrap(),
        Some(context)
    );
}

#[tokio::test]
async fn session_requires_complete_observed_v2_authority() {
    let (store, captured, context) = ready().await;
    assert!(store
        .session_install("fleet", "first", 1, 10)
        .await
        .is_err());
    assert!(store.session_get("fleet").await.unwrap().is_none());
    store
        .handoff_acknowledge(
            "fleet",
            PROFILE,
            1,
            Some(&serde_json::to_string(&context).unwrap()),
            &captured,
        )
        .await
        .unwrap();
    execute(
        &store,
        "DELETE FROM fleet_auth_context_history WHERE fleet_key='fleet'",
    )
    .await;
    assert!(store
        .session_install("fleet", "first", 1, 11)
        .await
        .is_err());
    assert!(store.session_get("fleet").await.unwrap().is_none());
}

#[tokio::test]
async fn legacy_session_without_observed_auth_does_not_reuse_an_old_reference() {
    let (store, _, _) = ready().await;
    execute(&store, "UPDATE github_auth_profile_revisions SET schema_version=1 WHERE profile_key='shared-github';
        INSERT INTO fleet_session_auth(fleet_key,profile_key,revision) VALUES('fleet','obsolete',99)").await;
    store
        .session_install("fleet", "replacement", 1, 10)
        .await
        .unwrap();
    let row = store
        .connection()
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT profile_key FROM fleet_session_auth WHERE fleet_key='fleet'",
        ))
        .await
        .unwrap();
    assert!(row.is_none());
}

#[tokio::test]
async fn exact_repository_admission_requires_the_promoted_numeric_proof() {
    let store = super::profiles::store().await;
    seed_v2_candidate(
        &store,
        1,
        &policy_json(&[r#"{"kind":"repository","owner":"example-org","repository":"repo"}"#]),
    )
    .await;
    store
        .auth_apply_full(
            PROFILE,
            1,
            true,
            None,
            5,
            Some(promotion(vec![binding("example-org", 100, 11)], 1, &[])),
        )
        .await
        .unwrap();
    let tx = store.begin().await.unwrap();
    assert!(store.auth_desired_context_tx(&tx, PROFILE, 1,
        r#"{"github":{"target":{"kind":"repository","owner":"example-org","repository":"repo"}}}"#).await.is_err());
}

#[tokio::test]
async fn missing_or_corrupt_active_snapshot_never_degrades_to_unpinned_admission() {
    for value in ["NULL", "'{broken'"] {
        let (store, _, _) = ready().await;
        execute(&store, &format!("UPDATE github_auth_profile_revisions SET validation_snapshot_json={value} WHERE profile_key='shared-github'")).await;
        let tx = store.begin().await.unwrap();
        assert!(store
            .auth_desired_context_tx(
                &tx,
                PROFILE,
                1,
                r#"{"github":{"target":{"kind":"organization","owner":"example-org"}}}"#
            )
            .await
            .is_err());
    }
}
