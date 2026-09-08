//! Read/admission fixture for the explicit HTTPS browser harness only.
//! Its seeded Active metadata is not a conformance or provisioning test.
use std::path::Path;

use sea_orm::{ConnectionTrait, Database, DatabaseBackend, Statement};
use serde_json::json;
use sha2::{Digest, Sha256};

pub async fn seed(directory: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let data = directory.join("data");
    std::fs::create_dir_all(&data)?;
    let database_path = data.join("shaula.db");
    let store = shaula_store::Store::open(&database_path).await?;
    store.migrate().await?;
    drop(store);

    let schema = json!({
        "type": "object", "additionalProperties": false, "required": ["runner_image"],
        "properties": {
            "runner_image": {"type": "string", "title": "Runner image", "enum": ["runner:approved", "runner:unapproved"]},
            "cpu_request": {"type": "string", "title": "CPU request", "enum": ["500m", "1", "2"]}
        }
    });
    let schema_bytes = serde_json::to_vec(&schema)?;
    let digest = format!("sha256:{}", hex::encode(Sha256::digest(&schema_bytes)));
    let artifact =
        shaula_core::artifact_layout::artifact_dir(&data.join("template-artifacts"), &digest)
            .ok_or("invalid test digest")?;
    std::fs::create_dir_all(artifact.join("schemas"))?;
    std::fs::write(
        artifact.join("schemas/parameters.schema.json"),
        schema_bytes,
    )?;

    // No credential, Fleet, worker, validation outbox or infrastructure state is seeded.
    // This fixture never invokes a platform or marks a real template conformant.
    let database = Database::connect(format!(
        "sqlite://{}?mode=rw",
        database_path.to_string_lossy().replace('\\', "/")
    ))
    .await?;
    database.execute(Statement::from_sql_and_values(
        DatabaseBackend::Sqlite,
        "INSERT INTO template_profiles (key,incarnation,desired_revision,active_revision,observed_revision,active_attestation_id,status,deletion_requested,created_at,updated_at) VALUES (?, ?, 1, 1, 1, 'browser-fixture-only', 'Active', 0, 1, 1)",
        ["browser-inputs".into(), "browser-template-incarnation".into()],
    )).await?;
    let policy = json!({"runner_image": ["runner:approved"], "cpu_request": ["500m", "1"]});
    database.execute(Statement::from_sql_and_values(
        DatabaseBackend::Sqlite,
        "INSERT INTO template_profile_revisions (profile_key,revision,artifact_digest,engine_ref,platform,bindings_contract,fleet_input_policy_json,state,created_at) VALUES ('browser-inputs',1,?,'terraform','kubernetes','shaula.bindings.kubernetes/v1',?,'Active',1)",
        [digest.into(), serde_json::to_string(&policy)?.into()],
    )).await?;
    database.close().await?;
    Ok(())
}
