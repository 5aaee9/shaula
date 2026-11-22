use sea_orm::{ConnectionTrait, DatabaseConnection};
use shaula_core::github::{GitHubTarget, Label, ScaleSetIdentity};
use shaula_core::plan::{admit_discover_plan, parse_plan};
use shaula_core::ports::{
    AccessFailure, EffectOutcome, GitHubAccessPort, JitConfig, LookupOutcome, PollOutcome,
    RemovalOutcome, RouteProof, RunnerLookup, RunnerRef, ScaleSetView, SessionHandle,
    TargetIdentity,
};
use shaula_core::template::{
    DiscoverOutput, DiscoverRoleResult, DiscoveredResource, ManagedResourceRole, ManifestReconcile,
};

use super::fixture::TestResult;

pub struct StubGitHub;

#[async_trait::async_trait]
impl GitHubAccessPort for StubGitHub {
    async fn resolve_runner_group(&self, _: &ScaleSetIdentity) -> Result<i64, AccessFailure> {
        Ok(7)
    }
    async fn lookup_scale_set(
        &self,
        i: &ScaleSetIdentity,
        _: i64,
    ) -> Result<LookupOutcome, AccessFailure> {
        Ok(LookupOutcome::ExactlyOne(ScaleSetView {
            id: 42,
            name: i.scale_set_name.clone(),
            runner_group_id: 7,
            runner_group_name: i.runner_group.clone(),
            labels: vec![Label::system("shaula-x64")],
        }))
    }
    async fn create_scale_set(
        &self,
        _: &ScaleSetIdentity,
        _: i64,
        _: &[Label],
    ) -> Result<EffectOutcome<ScaleSetView>, AccessFailure> {
        Err(AccessFailure::PermissionDenied)
    }
    async fn establish_session(
        &self,
        _: i64,
        _: &str,
    ) -> Result<EffectOutcome<SessionHandle>, AccessFailure> {
        Err(AccessFailure::PermissionDenied)
    }
    async fn delete_session(&self, _: i64, _: &str) -> Result<(), AccessFailure> {
        Ok(())
    }
    async fn poll_messages(
        &self,
        _: &SessionHandle,
        _: i64,
        _: i64,
    ) -> Result<PollOutcome, AccessFailure> {
        Ok(PollOutcome::NoMessage)
    }
    async fn ack_message(&self, _: &SessionHandle, _: i64) -> Result<(), AccessFailure> {
        Ok(())
    }
    async fn acquire_jobs(
        &self,
        _: i64,
        _: &SessionHandle,
        _: &[i64],
    ) -> Result<EffectOutcome<Vec<i64>>, AccessFailure> {
        Ok(EffectOutcome::Definite(Vec::new()))
    }
    async fn generate_jit(
        &self,
        _: i64,
        _: &str,
    ) -> Result<EffectOutcome<JitConfig>, AccessFailure> {
        Err(AccessFailure::PermissionDenied)
    }
    async fn get_runner_by_name(&self, _: i64, _: &str) -> Result<RunnerLookup, AccessFailure> {
        Ok(RunnerLookup::None)
    }
    async fn remove_runner(&self, _: i64) -> Result<RemovalOutcome, AccessFailure> {
        Ok(RemovalOutcome::AlreadyAbsent)
    }
    async fn list_runners(&self, _: i64) -> Result<Vec<RunnerRef>, AccessFailure> {
        Ok(Vec::new())
    }
    async fn resolve_target_identity(
        &self,
        _: &GitHubTarget,
    ) -> Result<TargetIdentity, AccessFailure> {
        Ok(TargetIdentity {
            organization_id: Some(1),
            repository_id: None,
            repository_owner_id: None,
        })
    }
    async fn ensure_route_proof(&self) -> Result<RouteProof, AccessFailure> {
        Ok(RouteProof {
            checked_at_unix_ms: 0,
            valid_until_unix_ms: i64::MAX,
            installation_id: 1,
            account_id: 1,
            organization_id: Some(1),
            repository_id: None,
            repository_owner_id: None,
        })
    }
}

pub fn verify_static_reconcile_guards() -> TestResult {
    assert!(ManifestReconcile {
        discover_module: "../escape".into(),
        settle_delay_secs: 0,
    }
    .validate()
    .is_err());
    assert!(ManifestReconcile {
        discover_module: "reconcile".into(),
        settle_delay_secs: 601,
    }
    .validate()
    .is_err());

    let bad_dir = tempfile::tempdir()?;
    std::fs::create_dir_all(bad_dir.path().join("reconcile"))?;
    let valid_manifest = shaula_template::manifest::parse_manifest(
        "api_version: shaula.io/template-profile/v1\nkind: RunnerTemplateProfile\nplatform: proxmox\n\
         vm_image_contract: shaula.proxmox-template/v1\nruntime:\n  protocol: terraform-cli/v1\n\
         \x20 engine: terraform\n  root_module: .\n  required_version: \">= 1.9, < 2.0\"\n\
         bindings_contract: shaula.bindings.proxmox/v1\nschemas:\n  bindings: schemas/bindings.schema.json\n\
         \x20 parameters: schemas/parameters.schema.json\nmanaged_resource_shape:\n\
         \x20 - role: runner\n    terraform_type: proxmox_qemu_vm\n    exact_count: 1\n\
         runner_image_digests: []\nruntime_policy_digest: sha256:p\nreconcile:\n  discover_module: reconcile\n",
    )?;
    std::fs::write(
        bad_dir.path().join("reconcile/main.tf"),
        "variable \"shaula\" { type = any }\nresource \"null_resource\" \"x\" {}\noutput \"shaula_discover\" { value = {} }\n",
    )?;
    assert!(
        shaula_template::reconcile_validate::verify_reconcile_module(
            bad_dir.path(),
            &valid_manifest
        )
        .is_err()
    );

    let mutating_plan = serde_json::json!({
        "format_version": "1.2",
        "applyable": true,
        "complete": true,
        "errored": false,
        "resource_changes": [{
            "address": "proxmox_qemu_vm.runner",
            "mode": "managed",
            "type": "proxmox_qemu_vm",
            "change": { "actions": ["create"] }
        }]
    });
    assert!(admit_discover_plan(&parse_plan(&mutating_plan)?).is_err());

    let shape = vec![
        ManagedResourceRole {
            role: "bootstrap".into(),
            terraform_type: "proxmox_nocloud_iso".into(),
            exact_count: 1,
        },
        ManagedResourceRole {
            role: "runner".into(),
            terraform_type: "proxmox_qemu_vm".into(),
            exact_count: 1,
        },
    ];
    let missing_role = DiscoverOutput {
        contract_version: 1,
        generation_id: "g1".into(),
        roles: vec![DiscoverRoleResult {
            role: "runner".into(),
            resources: vec![],
        }],
    };
    assert!(missing_role.validate("g1", &shape).is_err());
    let bad_import = DiscoverOutput {
        contract_version: 1,
        generation_id: "g1".into(),
        roles: vec![
            DiscoverRoleResult {
                role: "bootstrap".into(),
                resources: vec![],
            },
            DiscoverRoleResult {
                role: "runner".into(),
                resources: vec![DiscoveredResource {
                    address: "proxmox_qemu_vm.runner".into(),
                    import_id: Some("-flag-injection".into()),
                }],
            },
        ],
    };
    assert!(bad_import.validate("g1", &shape).is_err());
    Ok(())
}

pub async fn seed_registry_rows(
    db: &DatabaseConnection,
    artifact_root: &std::path::Path,
    reconcile_digest: &str,
    plain_digest: &str,
) -> TestResult {
    let reconcile_yaml = std::fs::read_to_string(
        shaula_core::artifact_layout::artifact_dir(artifact_root, reconcile_digest)
            .ok_or("artifact dir")?
            .join("profile.yaml"),
    )?;
    let plain_yaml = std::fs::read_to_string(
        shaula_core::artifact_layout::artifact_dir(artifact_root, plain_digest)
            .ok_or("artifact dir")?
            .join("profile.yaml"),
    )?;
    let auth_ctx = serde_json::to_string(&shaula_core::auth_context::ResolvedAuthContext {
        profile_key: "prod-app".into(),
        revision: 1,
        github_host: "github.com".into(),
        target: GitHubTarget::organization("example-org")?,
        app_id: "42".into(),
        installation_id: 1,
        account_id: 1,
        account_kind: shaula_core::auth_policy::AccountKind::Organization,
        login: "example-org".into(),
        organization_id: Some(1),
        repository_id: None,
        repository_owner_id: None,
    })?;
    for stmt in [
        "INSERT INTO github_auth_profiles(key,incarnation,desired_revision,active_revision,status,deletion_requested,created_at,updated_at) VALUES('prod-app','inc-a',1,1,'Active',0,1,1)",
        "INSERT INTO github_auth_profile_revisions(profile_key,revision,kind,app_id,allowlist_json,credential_bytes,state,created_at,schema_version,policy_json) VALUES('prod-app',1,'github_app','42','[]',X'00','Active',1,2,'{\"selectors\":[{\"kind\":\"organization\",\"owner\":\"example-org\"}]}')",
        "INSERT INTO template_profiles(key,incarnation,desired_revision,active_revision,status,deletion_requested,created_at,updated_at) VALUES('pve-reconcile','inc-t1',1,1,'Active',0,1,1)",
        "INSERT INTO template_profiles(key,incarnation,desired_revision,active_revision,status,deletion_requested,created_at,updated_at) VALUES('pve-plain','inc-t2',1,1,'Active',0,1,1)",
        "INSERT INTO fleets(key,incarnation,desired_revision,observed_revision,mutation_fence,deletion_marker,phase,tombstone,created_at,updated_at) VALUES('f1','inc-f1',1,1,1,0,'Ready',0,1,1)",
        "INSERT INTO fleet_auth_handoffs(fleet_key,desired_profile_key,desired_revision,observed_profile_key,observed_revision,state,cleanup_only,attempts) VALUES('f1','prod-app',1,'prod-app',1,'Observed',0,0)",
    ] {
        db.execute_unprepared(stmt).await?;
    }
    db.execute(sea_orm::Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        "INSERT INTO fleet_auth_contexts(fleet_key,desired_profile_key,desired_revision,desired_fence,desired_context_json,observed_profile_key,observed_revision,observed_context_json,state,attempts,updated_at) VALUES('f1','prod-app',1,1,?,'prod-app',1,?,'Observed',0,1)",
        vec![auth_ctx.clone().into(), auth_ctx.into()],
    ))
    .await?;
    let insert_rev = |key: &str, digest: &str, yaml: &str| {
        sea_orm::Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "INSERT INTO template_profile_revisions(profile_key,revision,artifact_digest,engine_ref,platform,bindings_contract,manifest_json,bindings_json,bindings_digest,fleet_input_policy_json,state,created_at) VALUES(?,1,?,'terraform','proxmox','shaula.bindings.proxmox/v1',?,'{}','bd1_test','{}','Active',1)",
            vec![key.into(), digest.into(), yaml.into()],
        )
    };
    db.execute(insert_rev(
        "pve-reconcile",
        reconcile_digest,
        &reconcile_yaml,
    ))
    .await?;
    db.execute(insert_rev("pve-plain", plain_digest, &plain_yaml))
        .await?;
    let fleet_spec = serde_json::json!({
        "kind": "github",
        "github": {
            "target": {"kind": "organization", "owner": "example-org"},
            "runner_group": "Default",
            "scale_set_name": "shaula-x64",
            "auth_profile_ref": "prod-app"
        },
        "capacity": {"min_runners": 0, "max_runners": 10},
        "template_profile_ref": {"profile_key": "pve-reconcile"},
        "template_inputs": {}
    })
    .to_string();
    db.execute(sea_orm::Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        "INSERT INTO fleet_revisions(fleet_key,incarnation,revision,spec_json,template_profile_key,template_revision,template_artifact_digest,template_attestation_id,auth_desired_profile_key,auth_desired_revision,inputs_digest,created_at) VALUES('f1','inc-f1',1,?,'pve-reconcile',1,?,'att','prod-app',1,'inputs',1)",
        vec![fleet_spec.into(), reconcile_digest.into()],
    ))
    .await?;
    Ok(())
}
