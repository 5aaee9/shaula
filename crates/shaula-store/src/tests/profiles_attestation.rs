//! Attestation durability tests: stale/verdict commits stay durable and
//! audited (R6-08), split to respect the 400-line limit (AGENTS.md).
#![allow(clippy::unwrap_used)]

use crate::store::Store;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use shaula_core::registry::ControlPlaneStore;

pub(crate) async fn store() -> Store {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("p2b.db");
    std::mem::forget(tmp);
    let s = Store::open(&path).await.unwrap();
    s.migrate().await.unwrap();
    s
}

#[tokio::test]
async fn stale_attestation_commits_durable_evidence_and_audit() {
    let store = store().await;
    let control_plane = crate::registry_impl::SqliteControlPlane::new(
        store.clone(),
        "target/test-artifacts".into(),
    );

    // Seed revision 1 and activate it (att A).
    let tx = store.begin().await.unwrap();
    store
        .template_commit_revision(
            &tx,
            shaula_core::template::TemplateRevisionInsert {
                key: "tpl".into(),
                incarnation: "inc-1".into(),
                revision: 1,
                artifact_digest: "sha256:a".into(),
                engine_ref: "terraform".into(),
                bindings_json: None,
                bindings_digest: Some("bd1_x".into()),
                fleet_input_policy_json: None,
            },
            1,
        )
        .await
        .unwrap();
    store
        .profile_change_insert(
            &tx,
            shaula_core::registry::ProfileChangeInsert {
                id: "pc-1".into(),
                resource_kind: "template_profile".into(),
                profile_key: "tpl".into(),
                revision: Some(1),
                kind: "publish".into(),
                now: 1,
            },
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    store
        .template_revision_validated(shaula_core::template::TemplateValidationRecord {
            key: "tpl".into(),
            revision: 1,
            ready: true,
            platform: "kubernetes".into(),
            bindings_contract: "shaula.bindings.kubernetes/v1".into(),
            manifest_json: "{}".into(),
            lock_digest: "sha256:lock".into(),
            reason: None,
        })
        .await
        .unwrap();

    let record = |key: &str| shaula_core::registry::AttestationRecord {
        id: shaula_core::registry::attestation_record_id("tpl", 1, key),
        attestation_key: key.into(),
        profile_key: "tpl".into(),
        revision: 1,
        subject_json: format!("{{\"k\":\"{key}\"}}"),
        subject_digest: format!("sha256:{key}"),
        result: "passed".into(),
        evidence_digest: None,
        suite: ("suite".into(), "v1".into()),
        completed_at: 1,
        subject_verified: true,
        activate: true,
    };
    let committed = control_plane
        .commit_attestation(record("att-a"), "ops".into(), 5)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(committed, shaula_core::registry::AttestationCommit::Created);

    // Advance the desired head to r2, then submit a LATE but correct r1
    // attestation under a fresh key (R6-08).
    let tx = store.begin().await.unwrap();
    store
        .template_commit_revision(
            &tx,
            shaula_core::template::TemplateRevisionInsert {
                key: "tpl".into(),
                incarnation: "inc-1".into(),
                revision: 2,
                artifact_digest: "sha256:a".into(),
                engine_ref: "terraform".into(),
                bindings_json: None,
                bindings_digest: Some("bd1_x".into()),
                fleet_input_policy_json: None,
            },
            6,
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let committed = control_plane
        .commit_attestation(record("att-a-late"), "ops".into(), 7)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        committed,
        shaula_core::registry::AttestationCommit::RecordedNotActivated,
        "a stale activation refusal is a verdict, not a storage failure"
    );

    // The late attestation row SURVIVED (durable evidence) and the audit
    // trail carries both facts.
    let attestations = crate::entities::template::template_conformance_attestations::Entity::find()
        .all(store.connection())
        .await
        .unwrap();
    assert_eq!(attestations.len(), 2, "both records stay durable");
    let audits = crate::entities::shared::audit_records::Entity::find()
        .filter(crate::entities::shared::audit_records::Column::Action.eq("attest"))
        .all(store.connection())
        .await
        .unwrap();
    assert_eq!(audits.len(), 2, "both attestations are audited");
    let profile = store.template_profile_get("tpl").await.unwrap().unwrap();
    assert_eq!(profile.desired_revision, 2);
    assert_eq!(profile.active_revision, Some(1));
    assert_eq!(
        profile.active_attestation_id.as_deref(),
        Some(shaula_core::registry::attestation_record_id("tpl", 1, "att-a").as_str()),
        "the frozen id is the STABLE composite identity, not the raw URI key"
    );
}
