//! Corrupt evidence and unsupported-session rejection regressions.
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
    assert!(
        super::session_support::install(&store, "fleet", "first", 1, &context, 10)
            .await
            .is_err()
    );
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
    assert!(
        super::session_support::install(&store, "fleet", "first", 1, &context, 11)
            .await
            .is_err()
    );
    assert!(store.session_get("fleet").await.unwrap().is_none());
}

#[tokio::test]
async fn unsupported_session_cannot_replace_retained_authority() {
    let (store, captured, context) = ready().await;
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
    execute(&store, "UPDATE github_auth_profile_revisions SET schema_version=1 WHERE profile_key='shared-github';
        INSERT INTO fleet_session_auth(fleet_key,profile_key,revision) VALUES('fleet','obsolete',99)").await;
    assert!(
        super::session_support::install(&store, "fleet", "replacement", 1, &context, 10)
            .await
            .is_err()
    );
    assert!(store.session_get("fleet").await.unwrap().is_none());
    let row = store
        .connection()
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT profile_key FROM fleet_session_auth WHERE fleet_key='fleet'",
        ))
        .await
        .unwrap();
    assert_eq!(
        row.unwrap().try_get::<String>("", "profile_key").unwrap(),
        "obsolete"
    );
}

#[tokio::test]
async fn missing_handoff_cannot_create_or_replace_a_session() {
    let (store, captured, context) = ready().await;
    assert!(
        super::session_support::install(&store, "missing", "new", 1, &context, 10)
            .await
            .is_err()
    );
    assert!(store.session_get("missing").await.unwrap().is_none());
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
    super::session_support::install(&store, "fleet", "original", 1, &context, 10)
        .await
        .unwrap();
    execute(
        &store,
        "DELETE FROM fleet_auth_handoffs WHERE fleet_key='fleet'",
    )
    .await;
    assert!(
        super::session_support::install(&store, "fleet", "replacement", 1, &context, 11)
            .await
            .is_err()
    );
    let session = store.session_get("fleet").await.unwrap().unwrap();
    assert_eq!(session.session_id, "original");
    assert_eq!(session.epoch, 1);
    let retained = store
        .connection()
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT profile_key,revision FROM fleet_session_auth WHERE fleet_key='fleet'",
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        retained.try_get::<String>("", "profile_key").unwrap(),
        PROFILE
    );
    assert_eq!(retained.try_get::<i64>("", "revision").unwrap(), 1);
}

#[tokio::test]
async fn unsupported_execution_refs_remain_readable_without_becoming_authority() {
    let (store, captured, context) = ready().await;
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
    super::session_support::install(&store, "fleet", "historical-session", 1, &context, 10)
        .await
        .unwrap();
    execute(&store, "UPDATE github_auth_profile_revisions SET schema_version=1, policy_json='{old-policy' WHERE profile_key='shared-github';
        UPDATE fleet_auth_context_history SET context_json='{old-context' WHERE fleet_key='fleet'").await;
    let dependents = store.auth_live_dependents(PROFILE).await.unwrap();
    assert_eq!(dependents.len(), 1);
    assert_eq!(dependents[0].fleet_key, "fleet");
    assert_eq!(dependents[0].retained_refs, vec![(PROFILE.into(), 1)]);
    assert!(dependents[0].retained_contexts.is_empty());
    let target: serde_json::Value = serde_json::from_str(&dependents[0].target_json).unwrap();
    assert_eq!(target["owner"], "example-org");
    assert!(store
        .auth_execution_context_get("fleet", PROFILE, 1)
        .await
        .is_err());
    let session = store.session_get("fleet").await.unwrap().unwrap();
    assert_eq!(session.session_id, "historical-session");
    let history = store
        .connection()
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT context_json FROM fleet_auth_context_history WHERE fleet_key='fleet'",
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        history.try_get::<String>("", "context_json").unwrap(),
        "{old-context"
    );
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
