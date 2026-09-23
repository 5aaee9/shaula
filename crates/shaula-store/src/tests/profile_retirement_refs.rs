use super::{forgejo_pool_support::TestResult, profile_retirement_support::*};
use sea_orm::ConnectionTrait;
use shaula_core::{
    lifecycle::GenerationState as G,
    registry::{ControlPlaneStore, LifecycleStore},
};

#[tokio::test]
async fn retained_session_and_uncertain_acquisition_each_prevent_auth_retirement() -> TestResult {
    for (setup, release) in [
        ("INSERT INTO fleet_sessions (fleet_key,session_id,epoch,scale_set_id,last_message_id,created_at)
            VALUES ('fleet','session',1,1,0,1);
          INSERT INTO fleet_session_auth (fleet_key,profile_key,revision) VALUES ('fleet','forgejo',1)",
         "DELETE FROM fleet_sessions; DELETE FROM fleet_session_auth"),
        ("INSERT INTO listener_messages (fleet_key,epoch,message_id,payload_digest,incarnation,fleet_revision,
            mutation_fence,profile_key,auth_revision,context_json,acked,created_at,updated_at)
            VALUES ('fleet',1,1,'digest','inc',1,1,'forgejo',1,'{}',0,1,1);
          INSERT INTO listener_acquisitions (fleet_key,epoch,message_id,runner_request_id,state,updated_at)
            VALUES ('fleet',1,1,1,'Uncertain',1)",
         "UPDATE listener_acquisitions SET state='Completed'"),
    ] {
        let fixture = fixture().await?;
        fixture.store.store().connection().execute_unprepared(setup).await?;
        fixture.store.fleet_set_tombstone("fleet", 2).await?;
        retire(&fixture).await?;
        fixture.store.periodic_scan(11).await?;
        assert_eq!(fixture.store.auth_profile_get("forgejo").await?.ok_or("missing auth")?.status, "Retiring");
        assert_eq!(fixture.store.template_profile_get("profile").await?.ok_or("missing template")?.status, "Retired");
        // These are durable recovery fixtures, not fabricated network acknowledgements.
        fixture.store.store().connection().execute_unprepared(release).await?;
        fixture.store.periodic_scan(12).await?;
        status(&fixture, "Retired").await?;
    }
    Ok(())
}

#[tokio::test]
async fn http_worker_authority_outlives_destroyed_and_blocks_retirement() -> TestResult {
    let fixture = fixture().await?;
    fixture.seed(G::Idle).await?;
    for state in [G::Retiring, G::DestroyPending, G::Destroying, G::Destroyed] {
        fixture
            .store
            .generation_advance("generation", state, 2)
            .await?;
    }
    fixture.store.store().connection().execute_unprepared(
        "INSERT INTO generation_http_state (generation_id,worker_epoch,worker_attempt,capability_hash)
         VALUES ('generation',1,'attempt',zeroblob(32))"
    ).await?;
    fixture.store.fleet_set_tombstone("fleet", 3).await?;
    retire(&fixture).await?;
    fixture.store.periodic_scan(11).await?;
    status(&fixture, "Retiring").await?;
    fixture
        .store
        .store()
        .connection()
        .execute_unprepared("UPDATE generation_http_state SET revoked=1")
        .await?;
    fixture.store.periodic_scan(12).await?;
    status(&fixture, "Retired").await
}

#[tokio::test]
async fn frozen_shared_pool_revision_blocks_retirement_after_pool_head_moves() -> TestResult {
    let fixture = fixture().await?;
    assert!(fixture
        .store
        .commit_template_pool_mutation(pool(&fixture)?)
        .await?
        .is_ok());
    fixture.store.store().connection().execute_unprepared(
        "UPDATE fleet_revisions SET template_profile_key=NULL,template_pool_ref='pool',template_pool_revision=1;
         UPDATE template_pools SET desired_revision=2;
         INSERT INTO template_pool_revisions (pool_key,revision,spec_json,failure_policy,created_at)
            VALUES ('pool',2,'{}','backpressure',2)"
    ).await?;
    retire(&fixture).await?;
    fixture.store.periodic_scan(11).await?;
    status(&fixture, "Retiring").await?;
    fixture.store.fleet_set_tombstone("fleet", 12).await?;
    fixture.store.periodic_scan(13).await?;
    status(&fixture, "Retired").await
}
