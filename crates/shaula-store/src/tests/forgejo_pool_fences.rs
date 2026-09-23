//! New shared routes must recheck pool/profile retirement under the commit writer.
use super::{
    forgejo_pool_support::TestResult,
    profile_retirement_support::{facts, fixture, pool},
};
use shaula_core::registry::{ControlPlaneStore, MutationError};

#[tokio::test]
async fn shared_pool_fleet_commit_rechecks_retirement_after_dependency_resolution() -> TestResult {
    for delete_pool in [true, false] {
        let fixture = fixture().await?;
        let mut pool = pool(&fixture)?;
        assert!(fixture
            .store
            .commit_template_pool_mutation(pool.clone())
            .await?
            .is_ok());
        let mut admitted = facts(false);
        admitted.resource_kind = "fleet";
        admitted.resource_key = "new-fleet".into();
        admitted.incarnation = "new-inc".into();
        admitted.auth_desired = Some(("forgejo".into(), 1));
        admitted.template_pool_ref = Some(("pool".into(), 1));
        let mut spec = fixture.spec.clone();
        spec.template_profile_ref = Default::default();
        spec.template_pool_ref = Some("pool".into());
        admitted.spec_json = serde_json::to_string(&spec)?;
        admitted.change.id = "fleet-put".into();
        admitted.change.kind = "Create".into();
        if delete_pool {
            pool.revision = 2;
            pool.change.id = "pool-delete".into();
            pool.change.kind = "Delete".into();
            assert!(fixture
                .store
                .commit_template_pool_delete(pool)
                .await?
                .is_ok());
        } else {
            assert!(fixture
                .store
                .commit_profile_retirement(facts(true))
                .await?
                .is_ok());
        }
        assert!(matches!(
            fixture.store.commit_fleet_mutation(admitted).await?,
            Err(MutationError::RetirementBlocked { .. })
        ));
        assert!(fixture.store.fleet_get("new-fleet").await?.is_none());
        assert!(fixture.store.fleet_change_get("fleet-put").await?.is_none());
    }
    Ok(())
}
