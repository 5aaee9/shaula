//! Fleet/admission/epoch/observation persistence tests.
#![allow(clippy::unwrap_used)]

use crate::store::Store;
pub(crate) async fn store() -> Store {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("p.db");
    std::mem::forget(tmp);
    let s = Store::open(&path).await.unwrap();
    s.migrate().await.unwrap();
    s
}

#[tokio::test]
async fn fleet_admission_commits_revision_change_audit_outbox() {
    let store = store().await;
    seed_auth_profile(&store, "production-app", "inc-1", 3).await;
    let tx = store.begin().await.unwrap();
    store
        .fleet_commit_revision(
            &tx,
            shaula_core::fleet::FleetRevisionInsert {
                key: "linux-x64".into(),
                incarnation: "inc-1".into(),
                revision: 1,
                spec_json: r#"{"key":"linux-x64"}"#.into(),
                template: Some((
                    "k8s-profile".into(),
                    4,
                    "sha256:artifact".into(),
                    "att-1".into(),
                )),
                auth_desired: ("production-app".into(), 3),
                inputs_digest: "sha256:inputs".into(),
                actor: "admin".into(),
                now: 1000,
            },
        )
        .await
        .unwrap();
    store
        .change_insert(&tx, "ch-1", "linux-x64", 1, "Create", 1000)
        .await
        .unwrap();
    store
        .audit_append(
            &tx,
            shaula_core::registry::AuditAppend {
                resource_kind: "fleet".into(),
                action: "create".into(),
                actor: "admin".into(),
                resource_key: "linux-x64".into(),
                revision: Some(1),
                outcome: "accepted".into(),
                detail_json: None,
                now: 1000,
            },
        )
        .await
        .unwrap();
    store
        .outbox_enqueue(&tx, "fleet", "fleet.change", r#"{"change":"ch-1"}"#, 1000)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let fleet = store.fleet_get("linux-x64").await.unwrap().unwrap();
    assert_eq!(fleet.desired_revision, 1);
    assert_eq!(fleet.mutation_fence, 1);
    assert_eq!(fleet.incarnation, "inc-1");

    let head = store
        .fleet_revision_latest("linux-x64")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(head.revision, 1);
    assert_eq!(head.template_profile_key.as_deref(), Some("k8s-profile"));
    assert_eq!(head.auth_desired_revision, 3);

    let flushed = store.outbox_flush(1001).await.unwrap();
    assert_eq!(flushed, 1);
}

#[tokio::test]
async fn incarnation_conflict_rejected() {
    let store = store().await;
    seed_auth_profile(&store, "a", "inc-1", 1).await;
    let tx = store.begin().await.unwrap();
    store
        .fleet_commit_revision(
            &tx,
            shaula_core::fleet::FleetRevisionInsert {
                key: "f".into(),
                incarnation: "inc-1".into(),
                revision: 1,
                spec_json: "{}".into(),
                template: None,
                auth_desired: ("a".into(), 1),
                inputs_digest: "d".into(),
                actor: "op".into(),
                now: 1,
            },
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let tx = store.begin().await.unwrap();
    let result = store
        .fleet_commit_revision(
            &tx,
            shaula_core::fleet::FleetRevisionInsert {
                key: "f".into(),
                incarnation: "inc-OTHER".into(),
                revision: 2,
                spec_json: "{}".into(),
                template: None,
                auth_desired: ("a".into(), 1),
                inputs_digest: "d".into(),
                actor: "op".into(),
                now: 2,
            },
        )
        .await;
    assert!(matches!(
        result,
        Err(crate::store::StoreError::Conflict { .. })
    ));
    tx.rollback().await.unwrap();

    // The failed mutation left no revision behind: the head is unchanged.
    let head = store.fleet_revision_latest("f").await.unwrap().unwrap();
    assert_eq!(head.revision, 1);
}

#[tokio::test]
async fn session_epoch_monotonic_across_reinstalls() {
    let store = store().await;
    assert_eq!(store.session_install("f1", "s1", 42, 1).await.unwrap(), 1);
    assert_eq!(store.session_install("f1", "s2", 42, 2).await.unwrap(), 2);
    // Different fleet namespaces independently (no cross-fleet coupling).
    assert_eq!(store.session_install("f2", "s1", 43, 2).await.unwrap(), 1);

    let session = store.session_get("f1").await.unwrap().unwrap();
    assert_eq!(session.epoch, 2);
    assert_eq!(session.session_id, "s2");
    assert_eq!(
        session.last_message_id, 0,
        "new session resets the checkpoint"
    );
}

#[tokio::test]
async fn demand_snapshot_overwrites_never_accumulates() {
    let store = store().await;
    store.demand_snapshot("f1", 4, 1).await.unwrap();
    store.demand_snapshot("f1", 2, 2).await.unwrap();
    store.demand_snapshot("f2", 9, 2).await.unwrap();

    let demand = store.demand_get("f1").await.unwrap().unwrap();
    assert_eq!(
        demand.total_assigned_jobs, 2,
        "snapshot replaces, never sums"
    );
    assert_eq!(
        store
            .demand_get("f2")
            .await
            .unwrap()
            .unwrap()
            .total_assigned_jobs,
        9
    );
}

#[tokio::test]
async fn duplicate_job_observations_are_idempotent() {
    let store = store().await;
    let first = store
        .observation_insert(shaula_core::registry::JobObservationInsert {
            fleet_key: "f1".into(),
            message_id: 5,
            observation_kind: "started".into(),
            runner_request_id: 11,
            job_id: "job-1".into(),
            runner_name: Some("r1".into()),
            now: 1,
        })
        .await
        .unwrap();
    let second = store
        .observation_insert(shaula_core::registry::JobObservationInsert {
            fleet_key: "f1".into(),
            message_id: 5,
            observation_kind: "started".into(),
            runner_request_id: 11,
            job_id: "job-1".into(),
            runner_name: Some("r1".into()),
            now: 2,
        })
        .await
        .unwrap();
    assert!(first);
    assert!(!second, "redelivery of the same message must dedupe");
}

#[tokio::test]
async fn artifact_registration_is_digest_idempotent() {
    let store = store().await;
    assert!(store
        .template_artifact_register("sha256:dup", 100, 1)
        .await
        .unwrap());
    assert!(!store
        .template_artifact_register("sha256:dup", 100, 2)
        .await
        .unwrap());
}

#[tokio::test]
async fn durable_format_marker_is_current_on_a_fresh_database() {
    use sea_orm::{ConnectionTrait, Statement};
    let store = store().await;
    let row = store
        .connection()
        .query_one(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "SELECT version FROM durable_format",
        ))
        .await
        .unwrap()
        .unwrap();
    let version: i64 = row.try_get("", "version").unwrap();
    assert_eq!(
        version,
        shaula_store_migration::m0008_auth_multi_account::DURABLE_FORMAT_VERSION_V2,
        "a fresh database is stamped the current format"
    );
}

#[tokio::test]
async fn legacy_durable_format_fails_closed_at_migration() {
    use sea_orm::{ConnectionTrait, Statement};
    let store = store().await;
    // Simulate a data directory written by an older pre-release build.
    store
        .connection()
        .execute(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "UPDATE durable_format SET version = 0",
        ))
        .await
        .unwrap();
    let error = store.migrate().await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("legacy durable identity encoding"),
        "a legacy database must be refused with the rebuild policy: {error}"
    );
}

/// Seeds an Active auth profile so fleet commits can resolve their auth
/// snapshot in-transaction (R9-03): the store now re-reads the CURRENT
/// active revision inside the commit, so the referenced profile row must
/// exist.
pub(crate) async fn seed_auth_profile(
    store: &Store,
    key: &str,
    incarnation: &str,
    active_revision: i64,
) {
    use crate::entities::auth::github_auth_profiles;
    use sea_orm::{ActiveValue::Set, EntityTrait};
    match github_auth_profiles::Entity::find_by_id(key.to_string())
        .one(store.connection())
        .await
        .unwrap()
    {
        Some(existing) => {
            let mut updated: github_auth_profiles::ActiveModel = existing.into();
            updated.active_revision = Set(Some(active_revision));
            updated.updated_at = Set(1);
            github_auth_profiles::Entity::update(updated)
                .exec(store.connection())
                .await
                .unwrap();
        }
        None => {
            let row = github_auth_profiles::ActiveModel {
                key: Set(key.to_string()),
                incarnation: Set(incarnation.to_string()),
                desired_revision: Set(active_revision),
                active_revision: Set(Some(active_revision)),
                observed_revision: Set(Some(active_revision)),
                status: Set("Active".to_string()),
                deletion_requested: Set(false),
                created_at: Set(1),
                updated_at: Set(1),
            };
            github_auth_profiles::Entity::insert(row)
                .exec(store.connection())
                .await
                .unwrap();
        }
    }
}
