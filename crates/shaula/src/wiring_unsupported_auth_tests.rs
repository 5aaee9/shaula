//! Historical and unknown auth encodings are never executable.

use crate::wiring::wiring_tests::{
    acknowledge_handoff, execution_wiring, seed_promoted_profile_and_fleet, test_plane, FLEET,
};
use shaula_core::registry::ControlPlaneStore;
use std::sync::Arc;

#[tokio::test]
async fn unsupported_saved_revision_cannot_build_credential_or_supervisor(
) -> Result<(), Box<dyn std::error::Error>> {
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
    for (schema, kind) in [(1, "github_app"), (3, "github_app"), (2, "pat")] {
        let plane = test_plane().await;
        let fence = seed_promoted_profile_and_fleet(&plane).await;
        acknowledge_handoff(&plane, fence).await;
        let url = format!(
            "sqlite://{}?mode=rw",
            plane.db_path.to_string_lossy().replace('\\', "/")
        );
        let db = sea_orm::Database::connect(url).await?;
        db.execute(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "UPDATE github_auth_profile_revisions SET schema_version = ?, kind = ?",
            [schema.into(), kind.into()],
        ))
        .await?;
        db.close().await?;
        let store: Arc<dyn ControlPlaneStore> = plane.control_plane.clone();
        let target = shaula_core::github::GitHubTarget::new_repository("5aaee9", "proj")?;
        assert!(crate::wiring_credential::build_credential(
            &store,
            crate::auth_worker_v2::tests::KEY,
            1,
            &target,
        )
        .await?
        .is_none());
        let mut wiring = execution_wiring(&plane).await;
        let fleet = store
            .fleet_get(FLEET)
            .await?
            .ok_or("fixture fleet missing")?;
        assert!(wiring
            .supervisor_for(FLEET, fleet.desired_revision, &fleet.phase)
            .await?
            .is_none());
    }
    Ok(())
}
