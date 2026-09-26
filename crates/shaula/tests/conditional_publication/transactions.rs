use sea_orm::ConnectionTrait;
use shaula_core::registry::{ControlPlaneStore, MutationError, Scope, TemplatePoolRegistryPort};

use super::{registry::*, support::*};

async fn counts(fixture: &Fixture) -> TestResult<[i64; 6]> {
    let mut result = [0; 6];
    for (index, table) in [
        "template_pool_revisions",
        "template_pool_members",
        "profile_changes",
        "audit_records",
        "outbox",
        "idempotency_records",
    ]
    .iter()
    .enumerate()
    {
        result[index] = fixture.count(table).await?;
    }
    Ok(result)
}

#[tokio::test]
async fn pool_noop_audits_without_an_idempotency_key_and_publishes_no_other_facts() -> TestResult {
    let fixture = Fixture::new().await?;
    let original = success(
        Kind::Pool
            .publish(
                &fixture.service,
                &actor(),
                &fixture.digest,
                0,
                Conditions::create(),
            )
            .await?,
    )?;
    let mut before = counts(&fixture).await?;
    let mut condition = Conditions::replace(&original, "unused")?;
    condition.idem = None;
    let noop = success(
        Kind::Pool
            .publish(&fixture.service, &actor(), &fixture.digest, 0, condition)
            .await?,
    )?;
    assert!(noop.no_op);
    assert_eq!(noop.etag, original.etag);
    before[3] += 1;
    assert_eq!(counts(&fixture).await?, before);
    Ok(())
}

#[tokio::test]
async fn pool_noop_audit_and_replay_record_roll_back_together_on_either_insert_failure(
) -> TestResult {
    let fixture = Fixture::new().await?;
    let original = success(
        Kind::Pool
            .publish(
                &fixture.service,
                &actor(),
                &fixture.digest,
                0,
                Conditions::create(),
            )
            .await?,
    )?;
    for table in ["audit_records", "idempotency_records"] {
        let before = counts(&fixture).await?;
        fixture.db.execute_unprepared(&format!(
            "CREATE TRIGGER fail_noop BEFORE INSERT ON {table} WHEN NEW.resource_kind = 'template_pool' BEGIN SELECT RAISE(ABORT, 'injected write failure'); END;"
        )).await?;
        let outcome = Kind::Pool
            .publish(
                &fixture.service,
                &actor(),
                &fixture.digest,
                0,
                Conditions::replace(&original, "failed-noop")?,
            )
            .await;
        assert!(
            outcome.is_err(),
            "{table}: failed transaction must not be accepted"
        );
        assert_eq!(
            counts(&fixture).await?,
            before,
            "{table}: partial facts escaped"
        );
        fixture
            .db
            .execute_unprepared("DROP TRIGGER fail_noop")
            .await?;
    }
    // Same key remains usable after rollback: no partial replay record survived.
    assert!(
        success(
            Kind::Pool
                .publish(
                    &fixture.service,
                    &actor(),
                    &fixture.digest,
                    0,
                    Conditions::replace(&original, "failed-noop")?
                )
                .await?
        )?
        .no_op
    );
    Ok(())
}

#[tokio::test]
async fn pool_noop_commit_rechecks_revision_incarnation_and_deletion_after_admission() -> TestResult
{
    let fixture = Fixture::new().await?;
    let original = success(
        Kind::Pool
            .publish(
                &fixture.service,
                &actor(),
                &fixture.digest,
                0,
                Conditions::create(),
            )
            .await?,
    )?;
    let head = fixture
        .store
        .template_pool_get(Kind::Pool.key())
        .await?
        .ok_or("pool head")?;
    let updated = success(
        Kind::Pool
            .publish(
                &fixture.service,
                &actor(),
                &fixture.digest,
                1,
                Conditions::replace(&original, "advance")?,
            )
            .await?,
    )?;
    let before = counts(&fixture).await?;
    for (incarnation, revision) in [(&*head.incarnation, 1), ("other-incarnation", 2)] {
        let result = fixture
            .store
            .commit_template_pool_noop(Kind::Pool.key(), incarnation, revision, &actor(), None, 123)
            .await?;
        assert!(matches!(
            result,
            Err(MutationError::PreconditionFailed { .. })
        ));
        assert_eq!(counts(&fixture).await?, before);
    }
    let mut retiring_actor = actor();
    retiring_actor.scopes.push(Scope::TemplateRetire);
    success(
        fixture
            .service
            .template_pool_delete(
                &retiring_actor,
                Kind::Pool.key(),
                Conditions::replace(&updated, "unused")?.expected,
                None,
            )
            .await?,
    )?;
    let before = counts(&fixture).await?;
    let result = fixture
        .store
        .commit_template_pool_noop(Kind::Pool.key(), &head.incarnation, 2, &actor(), None, 123)
        .await?;
    assert!(matches!(result, Err(MutationError::Gone { .. })));
    assert_eq!(counts(&fixture).await?, before);
    Ok(())
}

#[tokio::test]
async fn commit_failure_without_a_replay_winner_never_reports_success_or_leaks_facts() -> TestResult
{
    let fixture = Fixture::new().await?;
    for kind in KINDS {
        let base = success(
            kind.publish(
                &fixture.service,
                &actor(),
                &fixture.digest,
                0,
                Conditions::create(),
            )
            .await?,
        )?;
        let before = counts(&fixture).await?;
        fixture.db.execute_unprepared("CREATE TRIGGER fail_publication BEFORE INSERT ON outbox BEGIN SELECT RAISE(ABORT, 'injected write failure'); END;").await?;
        let result = kind
            .publish(
                &fixture.service,
                &actor(),
                &fixture.digest,
                1,
                Conditions::replace(&base, "failed-commit")?,
            )
            .await;
        assert!(
            result.is_err(),
            "{kind:?}: failed commit without winner must remain an error"
        );
        assert_eq!(kind.revision(&fixture).await?, 1);
        assert_eq!(
            counts(&fixture).await?,
            before,
            "{kind:?}: partial facts escaped"
        );
        fixture
            .db
            .execute_unprepared("DROP TRIGGER fail_publication")
            .await?;
        let recovered = success(
            kind.publish(
                &fixture.service,
                &actor(),
                &fixture.digest,
                1,
                Conditions::replace(&base, "failed-commit")?,
            )
            .await?,
        )?;
        assert_eq!(recovered.change.revision, 2);
    }
    Ok(())
}
