use super::forgejo_pool_support::{Fixture, TestResult};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use shaula_core::{
    forgejo::ForgejoScope,
    ports::forgejo::ForgejoAuthProbe,
    registry::{AuthPromotion, AuthPromotionOutcome, ControlPlaneStore},
};

fn probe(principal_id: Option<u64>) -> ForgejoAuthProbe {
    ForgejoAuthProbe {
        server_version: "16.0.4".into(),
        principal_id,
        target_id: None,
        checked_at_unix_ms: 10,
        valid_until_unix_ms: 60_010,
        runner_count: 0,
    }
}
async fn auth_fixture(principal: Option<u64>) -> TestResult<Fixture> {
    let mut fixture = Fixture::new(0, 1).await?;
    if principal.is_some() {
        fixture.spec.forgejo.as_mut().ok_or("section")?.scope = ForgejoScope::User;
        fixture
            .store
            .store()
            .connection()
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "UPDATE fleet_revisions SET spec_json=?",
                [serde_json::to_string(&fixture.spec)?.into()],
            ))
            .await?;
    }
    let target = fixture.spec.forgejo.as_ref().ok_or("section")?.target();
    let db = fixture.store.store().connection();
    db.execute_unprepared("INSERT INTO github_auth_profiles
        (key,incarnation,desired_revision,active_revision,observed_revision,status,deletion_requested,created_at,updated_at)
        VALUES ('forgejo','auth-inc',2,1,1,'Validating',0,1,1)").await?;
    for (revision, state) in [(1, "Active"), (2, "Validating")] {
        db.execute(Statement::from_sql_and_values(DatabaseBackend::Sqlite,
            "INSERT INTO github_auth_profile_revisions (profile_key,revision,kind,allowlist_json,credential_bytes,
             state,created_at,schema_version,policy_json,validation_snapshot_json)
             VALUES ('forgejo',?,'forgejo_token','[]',x'01',?,1,1,?,?)",
            [revision.into(),state.into(),serde_json::to_string(&target)?.into(),serde_json::to_string(&probe(principal))?.into()])).await?;
    }
    Ok(fixture)
}
async fn promote(fixture: &Fixture, principal: Option<u64>) -> TestResult<AuthPromotionOutcome> {
    Ok(fixture
        .store
        .auth_apply_validation_v2(
            "forgejo",
            2,
            true,
            None,
            10,
            Some(AuthPromotion {
                bindings: vec![],
                snapshot_json: serde_json::to_string(&probe(principal))?,
            }),
        )
        .await?)
}

#[tokio::test]
async fn token_rotation_appends_fleet_revision_and_fence_without_handoff() -> TestResult {
    let fixture = auth_fixture(None).await?;
    let previous = fixture
        .store
        .fleet_revision_latest("fleet")
        .await?
        .ok_or("revision")?;
    assert_eq!(
        promote(&fixture, None).await?,
        AuthPromotionOutcome::Promoted
    );
    let latest = fixture
        .store
        .fleet_revision_latest("fleet")
        .await?
        .ok_or("revision")?;
    assert_eq!(latest.revision, 2);
    assert_eq!(latest.auth_desired, ("forgejo".into(), 2));
    assert_eq!(
        latest.template_artifact_digest,
        previous.template_artifact_digest
    );
    assert_eq!(latest.spec_json, previous.spec_json);
    assert_eq!(
        fixture
            .store
            .fleet_get("fleet")
            .await?
            .ok_or("head")?
            .mutation_fence,
        2
    );
    assert!(fixture.store.handoff_get("fleet").await?.is_none());
    assert!(fixture
        .store
        .fleet_auth_context_get("fleet")
        .await?
        .is_none());
    Ok(())
}

#[tokio::test]
async fn user_token_rotation_cannot_switch_principals() -> TestResult {
    let fixture = auth_fixture(Some(42)).await?;
    assert_eq!(
        promote(&fixture, Some(43)).await?,
        AuthPromotionOutcome::Rejected
    );
    assert_eq!(
        fixture
            .store
            .auth_profile_get("forgejo")
            .await?
            .ok_or("profile")?
            .active_revision,
        Some(1)
    );
    assert_eq!(
        fixture
            .store
            .fleet_get("fleet")
            .await?
            .ok_or("head")?
            .desired_revision,
        1
    );
    let candidate = fixture
        .store
        .auth_revision_get("forgejo", 2)
        .await?
        .ok_or("candidate")?;
    assert_eq!(candidate.reason.as_deref(), Some("ForgejoPrincipalChanged"));
    Ok(())
}

#[tokio::test]
async fn user_token_rotation_preserves_verified_principal() -> TestResult {
    let fixture = auth_fixture(Some(42)).await?;
    assert_eq!(
        promote(&fixture, Some(42)).await?,
        AuthPromotionOutcome::Promoted
    );
    let active = fixture
        .store
        .auth_revision_get("forgejo", 2)
        .await?
        .ok_or("candidate")?;
    let saved: ForgejoAuthProbe = serde_json::from_str(
        active
            .validation_snapshot_json
            .as_deref()
            .ok_or("snapshot")?,
    )?;
    assert_eq!(saved.principal_id, Some(42));
    Ok(())
}

#[tokio::test]
async fn forgejo_auth_reads_keep_active_validation_and_candidate_separate() -> TestResult {
    use shaula_core::registry::{Actor, ProfileRegistryPort, Scope};
    let fixture = auth_fixture(Some(42)).await?;
    let service = shaula_daemon::service::ControlPlane::new(
        fixture.store.clone(),
        fixture.clock.clone(),
        vec![1; 32],
        10,
        fixture.directory.path().join("unused-terraform"),
    );
    let actor = Actor {
        name: "test".into(),
        scopes: vec![Scope::AuthRead],
    };
    let view = service
        .auth_get(&actor, "forgejo")
        .await?
        .map_err(|e| format!("read failed: {e:?}"))?;
    assert_eq!(view.kind, Some(shaula_core::auth::AuthKind::ForgejoToken));
    assert_eq!(view.status, "Validating");
    let active = view.active.ok_or("active")?;
    let candidate = view.desired.ok_or("candidate")?;
    assert_eq!(active.state, "Active");
    assert_eq!(candidate.state, "Validating");
    assert!(active.bindings.is_empty());
    assert!(active.target_policy.is_none());
    let forgejo = active.forgejo.ok_or("Forgejo metadata")?;
    assert_eq!(forgejo.target.scope, ForgejoScope::User);
    assert_eq!(
        forgejo.validation.ok_or("validation")?.principal_id,
        Some(42)
    );
    assert_eq!(view.live_fleets.len(), 1);
    assert!(view.live_fleets[0].target.is_none());
    assert_eq!(
        view.live_fleets[0]
            .forgejo_target
            .as_ref()
            .ok_or("target")?
            .scope,
        ForgejoScope::User
    );
    assert_eq!(service.auth_list(&actor).await?.len(), 1);
    Ok(())
}

#[tokio::test]
async fn token_activation_requires_fresh_supported_version_evidence() -> TestResult {
    for case in ["missing", "expired", "future", "old-version"] {
        let fixture = auth_fixture(None).await?;
        let mut evidence = probe(None);
        match case {
            "expired" => evidence.valid_until_unix_ms = 10,
            "future" => evidence.checked_at_unix_ms = 11,
            "old-version" => evidence.server_version = "14.0.3".into(),
            _ => {}
        }
        let promotion = if case == "missing" {
            None
        } else {
            Some(AuthPromotion {
                bindings: vec![],
                snapshot_json: serde_json::to_string(&evidence)?,
            })
        };
        assert_eq!(
            fixture
                .store
                .auth_apply_validation_v2("forgejo", 2, true, None, 10, promotion)
                .await?,
            AuthPromotionOutcome::Rejected,
            "{case}"
        );
        assert_eq!(
            fixture
                .store
                .auth_profile_get("forgejo")
                .await?
                .ok_or("profile")?
                .active_revision,
            Some(1)
        );
        assert_eq!(
            fixture
                .store
                .fleet_revision_latest("fleet")
                .await?
                .ok_or("fleet")?
                .revision,
            1
        );
    }
    Ok(())
}

#[tokio::test]
async fn deleting_forgejo_fleet_retains_credential_dependency() -> TestResult {
    let fixture = auth_fixture(None).await?;
    fixture
        .store
        .store()
        .connection()
        .execute_unprepared(
            "UPDATE fleets SET desired_revision=2,deletion_marker=1,phase='Deleting'",
        )
        .await?;
    let dependencies = fixture.store.auth_live_dependents("forgejo").await?;
    assert_eq!(dependencies.len(), 1);
    assert!(dependencies[0].retained_contexts.is_empty());
    let target: shaula_core::forgejo::ForgejoTarget =
        serde_json::from_str(&dependencies[0].target_json)?;
    assert_eq!(target.scope, ForgejoScope::Instance);
    assert_eq!(dependencies[0].revision, 2);
    Ok(())
}
