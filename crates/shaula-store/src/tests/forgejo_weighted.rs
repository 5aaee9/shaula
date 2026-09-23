//! Forgejo uses the same durable weighted route and caps as GitHub, not a job router.
use super::forgejo_pool_support::{Fixture, TestResult};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use shaula_core::{
    fleet::TemplateProfileRefDto,
    registry::{ControlPlaneStore, LifecycleStore},
};
use std::sync::atomic::Ordering;

async fn configure(fixture: &mut Fixture, shared: bool, policy: &str) -> TestResult {
    fixture.spec.template_profile_ref = TemplateProfileRefDto::default();
    let pool = serde_json::json!({"failure_policy":policy,"members":[
        {"key":"capped","template_profile_ref":"profile","weight":100,"max_runners":0},
        {"key":"available","template_profile_ref":"profile","weight":3,"max_runners":1,
            "template_inputs":{"runner_image":"code.forgejo.org/forgejo/runner:13.1.0"}}
    ]});
    if shared {
        fixture.spec.template_pool_ref = Some("shared".into());
    } else {
        fixture.spec.template_pool = Some(serde_json::from_value(pool.clone())?);
    }
    let db = fixture.store.store().connection();
    db.execute(Statement::from_sql_and_values(
        DbBackend::Sqlite,
        "UPDATE fleet_revisions SET spec_json=?,template_profile_key=NULL,template_revision=NULL,
            template_artifact_digest=NULL,template_attestation_id=NULL",
        [serde_json::to_string(&fixture.spec)?.into()],
    ))
    .await?;
    if shared {
        db.execute_unprepared("INSERT INTO template_pools
            (key,incarnation,desired_revision,observed_revision,phase,deletion_marker,tombstone,created_at,updated_at)
            VALUES ('shared','pool-inc',1,1,'Ready',0,0,1,1);
            UPDATE fleet_revisions SET template_pool_ref='shared',template_pool_revision=1").await?;
        db.execute(Statement::from_sql_and_values(DbBackend::Sqlite,
            "INSERT INTO template_pool_revisions (pool_key,revision,spec_json,failure_policy,created_at) VALUES ('shared',1,?,?,1)",
            [pool.to_string().into(),policy.into()],
        )).await?;
    }
    let (table, key_column, revision_column, key) = if shared {
        (
            "template_pool_members",
            "pool_key",
            "pool_revision",
            "shared",
        )
    } else {
        (
            "fleet_revision_pool_members",
            "fleet_key",
            "fleet_revision",
            "fleet",
        )
    };
    for (member, weight, cap, inputs) in [
        ("capped", 100, 0, "{}"),
        (
            "available",
            3,
            1,
            r#"{"runner_image":"code.forgejo.org/forgejo/runner:13.1.0"}"#,
        ),
    ] {
        db.execute(Statement::from_sql_and_values(DbBackend::Sqlite, format!(
            "INSERT INTO {table} ({key_column},{revision_column},member_key,template_profile_key,template_revision,
                template_artifact_digest,template_attestation_id,template_inputs_json,inputs_digest,weight,max_runners)
             VALUES (?,1,?,'profile',1,?,'attestation',?,'member-inputs',?,?)"),
            [key.into(),member.into(),format!("sha256:{}", "a".repeat(64)).into(),inputs.into(),weight.into(),cap.into()],
        )).await?;
    }
    Ok(())
}

#[tokio::test]
async fn forgejo_weighted_route_freezes_member_inputs_and_respects_caps_after_restart() -> TestResult
{
    for shared in [true, false] {
        let mut fixture = Fixture::new(2, 3).await?;
        configure(&mut fixture, shared, "redistribute").await?;
        assert_eq!(fixture.supervisor().await?.tick().await?.created, 1);
        let generation = fixture.generation().await?;
        assert_eq!(generation.pool_member_key.as_deref(), Some("available"));
        assert_eq!(generation.template_profile_key, "profile");
        assert_eq!(generation.inputs_digest, "member-inputs");
        assert_eq!(
            fixture
                .runtime
                .inputs
                .lock()
                .map_err(|_| "inputs poisoned")?
                .as_slice(),
            [serde_json::json!({"runner_image":"code.forgejo.org/forgejo/runner:13.1.0"})]
        );
        fixture.declare("idle")?;
        // A newly constructed supervisor must not redraw or ignore durable occupancy.
        assert_eq!(fixture.supervisor().await?.tick().await?.created, 0);
        assert_eq!(fixture.forgejo.posts.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.store.generations_for_fleet("fleet").await?.len(), 1);
        assert_eq!(fixture.store.generations_occupancy("fleet").await?, 1);
    }
    Ok(())
}

#[tokio::test]
async fn forgejo_inline_backpressure_does_not_register_when_any_member_is_capped() -> TestResult {
    let mut fixture = Fixture::new(1, 3).await?;
    configure(&mut fixture, false, "backpressure").await?;
    assert_eq!(fixture.supervisor().await?.tick().await?.created, 0);
    assert_eq!(fixture.forgejo.posts.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.store.generations_occupancy("fleet").await?, 0);
    Ok(())
}
