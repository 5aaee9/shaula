use sea_orm::{sea_query::Expr, ConnectionTrait, EntityTrait};

use crate::entities::{
    shared::{audit_records, outbox},
    template::{template_profile_revisions, template_profiles},
};
use crate::template_activation::{public_validation_reason, ValidationCommit};

use super::template_activation_support::{validated, Fixture, TestResult};

#[tokio::test]
async fn ready_upgrade_activates_once_with_atomic_provenance_and_change() -> TestResult {
    let fixture = Fixture::new().await?;
    let store = &fixture.store;
    template_profile_revisions::Entity::update_many()
        .col_expr(
            template_profile_revisions::Column::State,
            Expr::value("Ready"),
        )
        .exec(store.connection())
        .await?;
    let (profile, revision) = fixture.snapshot().await?;
    assert_eq!(
        store
            .template_validation_commit(&profile, &revision, Ok(validated()), 5)
            .await?,
        ValidationCommit::Activated
    );
    let (active, activated) = fixture.snapshot().await?;
    assert_eq!(active.active_revision, Some(1));
    assert_eq!(active.observed_revision, Some(1));
    assert_eq!(active.status, "Active");
    assert_eq!(activated.state, "Active");
    assert_eq!(activated.lock_digest.as_deref(), Some("sha256:lock"));
    let pin = active
        .active_attestation_id
        .as_deref()
        .ok_or("activation pin missing")?;
    assert!(pin.starts_with("static-validation-v1:"));
    assert_eq!(
        store
            .profile_change_get("publish-1")
            .await?
            .ok_or("change missing")?
            .state,
        "Converged"
    );
    let audits = audit_records::Entity::find()
        .all(store.connection())
        .await?;
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0].action, "activate");
    let detail = audits[0].detail_json.as_deref().ok_or("audit missing")?;
    assert!(!detail.contains("protected-value"));
    assert!(detail.contains(pin));
    assert_eq!(
        outbox::Entity::find().all(store.connection()).await?.len(),
        1
    );
    assert_eq!(
        store
            .template_validation_commit(&profile, &revision, Ok(validated()), 6)
            .await?,
        ValidationCommit::Superseded
    );
    assert_eq!(
        store
            .template_validation_commit(&profile, &revision, Err("ManifestInvalid"), 7)
            .await?,
        ValidationCommit::Superseded
    );
    assert_eq!(fixture.snapshot().await?.0, active);
    assert_eq!(
        audit_records::Entity::find()
            .all(store.connection())
            .await?
            .len(),
        1
    );
    Ok(())
}

#[tokio::test]
async fn rejected_upgrade_keeps_previous_pin_and_only_completes_its_publish() -> TestResult {
    let fixture = Fixture::new().await?;
    let (profile, revision) = fixture.snapshot().await?;
    fixture
        .store
        .template_validation_commit(&profile, &revision, Ok(validated()), 5)
        .await?;
    let previous = fixture.snapshot().await?.0;
    fixture.publish(2).await?;
    let tx = fixture.store.begin().await?;
    fixture
        .store
        .profile_change_insert(
            &tx,
            shaula_core::registry::ProfileChangeInsert {
                id: "unrelated-retire".into(),
                resource_kind: "template_profile".into(),
                profile_key: "tpl".into(),
                revision: Some(2),
                kind: "Retire".into(),
                now: 6,
            },
        )
        .await?;
    tx.commit().await?;
    let (profile, revision) = fixture.snapshot().await?;
    assert_eq!(
        fixture
            .store
            .template_validation_commit(&profile, &revision, Err("ManifestInvalid"), 7)
            .await?,
        ValidationCommit::Rejected
    );
    let (head, rejected) = fixture.snapshot().await?;
    assert_eq!(head.active_revision, Some(1));
    assert_eq!(head.active_attestation_id, previous.active_attestation_id);
    assert_eq!(head.status, "Rejected");
    assert_eq!(rejected.reason.as_deref(), Some("ManifestInvalid"));
    assert_eq!(
        fixture
            .store
            .profile_change_get("publish-1")
            .await?
            .ok_or("change missing")?
            .state,
        "Converged"
    );
    assert_eq!(
        fixture
            .store
            .profile_change_get("publish-2")
            .await?
            .ok_or("change missing")?
            .state,
        "Rejected"
    );
    assert_eq!(
        fixture
            .store
            .profile_change_get("unrelated-retire")
            .await?
            .ok_or("change missing")?
            .state,
        "Pending"
    );
    Ok(())
}

#[tokio::test]
async fn scan_result_cannot_cross_desired_incarnation_retirement_or_candidate_fences() -> TestResult
{
    for mutation in ["desired", "incarnation", "retired", "artifact", "state"] {
        let fixture = Fixture::new().await?;
        let (profile, revision) = fixture.snapshot().await?;
        match mutation {
            "desired" => fixture.publish(2).await?,
            "incarnation" => {
                template_profiles::Entity::update_many()
                    .col_expr(
                        template_profiles::Column::Incarnation,
                        Expr::value("other-inc"),
                    )
                    .exec(fixture.store.connection())
                    .await?;
            }
            "retired" => {
                template_profiles::Entity::update_many()
                    .col_expr(
                        template_profiles::Column::DeletionRequested,
                        Expr::value(true),
                    )
                    .col_expr(template_profiles::Column::Status, Expr::value("Retiring"))
                    .exec(fixture.store.connection())
                    .await?;
            }
            "artifact" => {
                template_profile_revisions::Entity::update_many()
                    .col_expr(
                        template_profile_revisions::Column::ArtifactDigest,
                        Expr::value("sha256:other"),
                    )
                    .exec(fixture.store.connection())
                    .await?;
            }
            _ => {
                template_profile_revisions::Entity::update_many()
                    .col_expr(
                        template_profile_revisions::Column::State,
                        Expr::value("Rejected"),
                    )
                    .exec(fixture.store.connection())
                    .await?;
            }
        }
        let before = fixture.snapshot().await?;
        for outcome in [Ok(validated()), Err("ManifestInvalid")] {
            assert_eq!(
                fixture
                    .store
                    .template_validation_commit(&profile, &revision, outcome, 8)
                    .await?,
                ValidationCommit::Superseded,
                "{mutation}"
            );
        }
        assert_eq!(fixture.snapshot().await?, before, "{mutation}");
        assert!(audit_records::Entity::find()
            .all(fixture.store.connection())
            .await?
            .is_empty());
    }
    Ok(())
}

#[tokio::test]
async fn audit_failure_rolls_back_activation_and_retry_commits_once() -> TestResult {
    let fixture = Fixture::new().await?;
    let (profile, revision) = fixture.snapshot().await?;
    fixture.store.connection().execute_unprepared(
        "CREATE TRIGGER reject_activation_audit BEFORE INSERT ON audit_records BEGIN SELECT RAISE(ABORT, 'test failure'); END"
    ).await?;
    assert!(fixture
        .store
        .template_validation_commit(&profile, &revision, Ok(validated()), 5)
        .await
        .is_err());
    assert_eq!(
        fixture.snapshot().await?,
        (profile.clone(), revision.clone())
    );
    assert_eq!(
        fixture
            .store
            .profile_change_get("publish-1")
            .await?
            .ok_or("change missing")?
            .state,
        "Pending"
    );
    assert!(outbox::Entity::find()
        .all(fixture.store.connection())
        .await?
        .is_empty());
    fixture
        .store
        .connection()
        .execute_unprepared("DROP TRIGGER reject_activation_audit")
        .await?;
    assert_eq!(
        fixture
            .store
            .template_validation_commit(&profile, &revision, Ok(validated()), 6)
            .await?,
        ValidationCommit::Activated
    );
    assert_eq!(
        audit_records::Entity::find()
            .all(fixture.store.connection())
            .await?
            .len(),
        1
    );
    Ok(())
}

#[tokio::test]
async fn historical_active_ready_revision_keeps_its_attestation_identity() -> TestResult {
    let fixture = Fixture::new().await?;
    template_profiles::Entity::update_many()
        .col_expr(template_profiles::Column::ActiveRevision, Expr::value(1))
        .col_expr(
            template_profiles::Column::ActiveAttestationId,
            Expr::value("historical-attestation"),
        )
        .col_expr(template_profiles::Column::Status, Expr::value("Active"))
        .exec(fixture.store.connection())
        .await?;
    template_profile_revisions::Entity::update_many()
        .col_expr(
            template_profile_revisions::Column::State,
            Expr::value("Ready"),
        )
        .exec(fixture.store.connection())
        .await?;
    let before = fixture.snapshot().await?;
    let scan = crate::registry_impl::SqliteControlPlane::new(
        fixture.store.clone(),
        fixture.temp.path().join("missing-artifacts"),
    );
    assert_eq!(scan.periodic_scan(10).await?.candidates_activated, 0);
    assert_eq!(fixture.snapshot().await?, before);
    assert!(audit_records::Entity::find()
        .all(fixture.store.connection())
        .await?
        .is_empty());
    Ok(())
}

#[test]
fn legacy_parser_errors_are_sanitized_for_revision_reads() {
    assert_eq!(
        public_validation_reason(Some("invalid YAML containing secret".into())).as_deref(),
        Some("ValidationFailed")
    );
    assert_eq!(
        public_validation_reason(Some("DependencyLockInvalid".into())).as_deref(),
        Some("DependencyLockInvalid")
    );
    assert_eq!(public_validation_reason(None), None);
}
