//! Read/admission fixture for the explicit HTTPS browser harness only.
//! A legacy Ready revision must activate through the real daemon startup scan.
//! This does not claim conformance or test provisioning.
use std::path::Path;

use sea_orm::{ConnectionTrait, Database, DatabaseBackend, Statement};
use serde_json::json;
use sha2::{Digest, Sha256};
use shaula_core::auth_context::{AccountBinding, RepositorySelection};
use shaula_core::auth_policy::{AccountKind, TargetPolicy, TargetSelector};
use shaula_core::registry::{
    auth_dependent_set_fingerprint, AuthIdentityProof, AuthPromotion, AuthPromotionOutcome,
    AuthValidationSnapshot, ControlPlaneStore,
};

pub async fn seed(directory: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let data = directory.join("data");
    std::fs::create_dir_all(&data)?;
    let database_path = data.join("shaula.db");
    let store = shaula_store::Store::open(&database_path).await?;
    store.migrate().await?;

    let schema = json!({
        "type": "object", "additionalProperties": false, "required": ["runner_image"],
        "properties": {
            "runner_image": {"type": "string", "title": "Runner image", "enum": ["runner:approved", "runner:unapproved"]},
            "cpu_request": {"type": "string", "title": "CPU request", "enum": ["500m", "1", "2"]}
        }
    });
    let sources = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates");
    let mut archive = tar::Builder::new(Vec::new());
    for (name, content) in [
        (
            "profile.yaml",
            std::fs::read(sources.join("kubernetes/profile.yaml"))?,
        ),
        (
            ".terraform.lock.hcl",
            std::fs::read(sources.join("kubernetes/.terraform.lock.hcl"))?,
        ),
        ("schemas/bindings.schema.json", b"{}".to_vec()),
        (
            "schemas/parameters.schema.json",
            serde_json::to_vec(&schema)?,
        ),
        (
            "main.tf",
            b"variable \"shaula\" {\n  type = any\n  sensitive = true\n}\n".to_vec(),
        ),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        archive.append_data(&mut header, name, content.as_slice())?;
    }
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    std::io::Write::write_all(&mut encoder, &archive.into_inner()?)?;
    let bytes = encoder.finish()?;
    let digest = format!("sha256:{}", hex::encode(Sha256::digest(&bytes)));
    store.artifact_archive_put(&digest, &bytes, 1).await?;
    drop(store);

    // Exercise real startup import from the bundled filesystem sources.
    let config_path = directory.join("bootstrap.json");
    let mut config: serde_json::Value = serde_json::from_slice(&std::fs::read(&config_path)?)?;
    config["template_source_dirs"] = json!([sources]);
    std::fs::write(config_path, serde_json::to_vec(&config)?)?;

    // No Fleet, validation outbox or infrastructure state is seeded. The Ready
    // Template exercises upgrade reconciliation without an attestation. The Active
    // auth fixture below has inert bytes and denies the browser test's target,
    // so real admission cannot create a Fleet or invoke GitHub or a platform.
    let database = Database::connect(format!(
        "sqlite://{}?mode=rw",
        database_path.to_string_lossy().replace('\\', "/")
    ))
    .await?;
    database.execute(Statement::from_sql_and_values(
        DatabaseBackend::Sqlite,
        "INSERT INTO template_profiles (key,incarnation,desired_revision,status,deletion_requested,created_at,updated_at) VALUES (?, ?, 1, 'Ready', 0, 1, 1)",
        ["browser-inputs".into(), "browser-template-incarnation".into()],
    )).await?;
    let policy = json!({"runner_image": ["runner:approved"], "cpu_request": ["500m", "1"]});
    database.execute(Statement::from_sql_and_values(
        DatabaseBackend::Sqlite,
        "INSERT INTO template_profile_revisions (profile_key,revision,artifact_digest,engine_ref,platform,bindings_contract,fleet_input_policy_json,state,created_at) VALUES ('browser-inputs',1,?,'terraform','kubernetes','shaula.bindings.kubernetes/v1',?,'Ready',1)",
        [digest.into(), serde_json::to_string(&policy)?.into()],
    )).await?;
    database.execute(Statement::from_string(
        DatabaseBackend::Sqlite,
        "INSERT INTO github_auth_profiles (key,incarnation,desired_revision,status,deletion_requested,created_at,updated_at) VALUES ('browser-auth','browser-auth-incarnation',1,'Validating',0,1,1)",
    )).await?;
    let policy = TargetPolicy::new(vec![TargetSelector::Organization {
        owner: "fixture-authorized-org".into(),
    }])?;
    database.execute(Statement::from_sql_and_values(
        DatabaseBackend::Sqlite,
        "INSERT INTO github_auth_profile_revisions (profile_key,revision,kind,app_id,schema_version,policy_json,allowlist_json,credential_bytes,state,created_at) VALUES ('browser-auth',1,'github_app','4863460',2,?,'',?,'Validating',1)",
        [serde_json::to_string(&policy)?.into(), b"browser-fixture-inert-credential".to_vec().into()],
    )).await?;
    database.close().await?;
    let store = shaula_store::Store::open(&database_path).await?;
    let control_plane =
        shaula_store::registry_impl::SqliteControlPlane::new(store, directory.join("artifacts"));
    let snapshot = AuthValidationSnapshot {
        candidate: ("browser-auth".into(), 1),
        dependent_set: auth_dependent_set_fingerprint(&[]),
        checked_fleets: vec![],
        identities: vec![AuthIdentityProof {
            login: "fixture-authorized-org".into(),
            account_id: 100,
            installation_id: 11,
            repositories: vec![],
        }],
    };
    // Supply the browser fixture's explicit identity proof to the real promotion
    // transaction. No Fleet exists, so there is no Fleet context to fabricate.
    let outcome = control_plane
        .auth_apply_validation_v2(
            "browser-auth",
            1,
            true,
            None,
            1,
            Some(AuthPromotion {
                bindings: vec![AccountBinding {
                    account_id: 100,
                    account_kind: AccountKind::Organization,
                    login: "fixture-authorized-org".into(),
                    installation_id: 11,
                    repository_selection: RepositorySelection::All,
                    validated_at_ms: 1,
                }],
                snapshot_json: serde_json::to_string(&snapshot)?,
            }),
        )
        .await?;
    if outcome != AuthPromotionOutcome::Promoted {
        return Err(std::io::Error::other("browser auth fixture was not promoted").into());
    }
    Ok(())
}
