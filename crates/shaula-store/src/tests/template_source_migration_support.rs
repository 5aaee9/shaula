use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sea_orm::{ConnectionTrait, DatabaseBackend, Statement, TransactionTrait};
use sha2::{Digest, Sha256};
use shaula_core::registry::TemplateSource;
use shaula_store_migration::{Migrator, MigratorTrait};

use crate::Store;

pub(super) type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

pub(super) struct Fixture {
    pub temp: tempfile::TempDir,
    pub store: Store,
    pub path: PathBuf,
    pub unique: TemplateSource,
}

impl Fixture {
    pub async fn legacy() -> TestResult<Self> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("sources.db");
        let store = Store::open(&path).await?;
        let tx = store.connection().begin().await?;
        Migrator::up(&tx, Some(14)).await?;
        tx.commit().await?;
        let unique = source(&store, "docker-legacy", b"unique archive").await?;
        let ambiguous = source(&store, "docker-a", b"ambiguous archive").await?;
        store
            .template_sources_replace(
                &[
                    unique.clone(),
                    TemplateSource {
                        key: "other-engine".into(),
                        engine_ref: "opentofu".into(),
                        ..unique.clone()
                    },
                    TemplateSource {
                        key: "other-platform".into(),
                        platform: "kubernetes".into(),
                        ..unique.clone()
                    },
                    ambiguous.clone(),
                    TemplateSource {
                        key: "docker-b".into(),
                        ..ambiguous.clone()
                    },
                ],
                1,
            )
            .await?;
        for (key, digest, engine, platform) in [
            (
                "unique",
                unique.artifact_digest.as_str(),
                "terraform",
                Some("docker"),
            ),
            (
                "second-unique",
                unique.artifact_digest.as_str(),
                "terraform",
                Some("docker"),
            ),
            (
                "ambiguous",
                ambiguous.artifact_digest.as_str(),
                "terraform",
                Some("docker"),
            ),
            (
                "absent",
                "sha256:no-matching-archive",
                "terraform",
                Some("docker"),
            ),
            (
                "wrong-engine",
                unique.artifact_digest.as_str(),
                "Terraform",
                Some("docker"),
            ),
            (
                "wrong-platform",
                unique.artifact_digest.as_str(),
                "terraform",
                Some("vm"),
            ),
            (
                "unknown-platform",
                unique.artifact_digest.as_str(),
                "terraform",
                None,
            ),
            (
                "empty-platform",
                unique.artifact_digest.as_str(),
                "terraform",
                Some(""),
            ),
        ] {
            seed_revision(&store, key, digest, engine, platform).await?;
        }
        seed_pins(&store, &unique.artifact_digest).await?;
        Ok(Self {
            temp,
            store,
            path,
            unique,
        })
    }

    pub async fn backup(&self, name: &str) -> TestResult<PathBuf> {
        let path = self.temp.path().join(name);
        self.store
            .connection()
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "VACUUM INTO ?",
                vec![path.to_string_lossy().to_string().into()],
            ))
            .await?;
        Ok(path)
    }
}

pub(super) async fn source(store: &Store, key: &str, bytes: &[u8]) -> TestResult<TemplateSource> {
    let digest = format!("sha256:{}", hex::encode(Sha256::digest(bytes)));
    store.artifact_archive_put(&digest, bytes, 1).await?;
    Ok(TemplateSource {
        key: key.into(),
        artifact_digest: digest,
        engine_ref: "terraform".into(),
        platform: "docker".into(),
    })
}

async fn seed_revision(
    store: &Store,
    key: &str,
    digest: &str,
    engine: &str,
    platform: Option<&str>,
) -> TestResult {
    store
        .connection()
        .execute(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "INSERT INTO template_profiles
         (key, incarnation, desired_revision, active_revision, observed_revision,
          active_attestation_id, status, deletion_requested, created_at, updated_at)
         VALUES (?, 'original-incarnation', 1, 1, 1, 'attestation', 'Active', 0, 1, 2)",
            vec![key.into()],
        ))
        .await?;
    store
        .connection()
        .execute(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "INSERT INTO template_profile_revisions
         (profile_key, revision, artifact_digest, engine_ref, platform, bindings_contract,
          manifest_json, lock_digest, bindings_json, bindings_digest, fleet_input_policy_json,
          state, reason, created_at)
         VALUES (?, 1, ?, ?, ?, 'v1', 'original manifest', 'original-lock',
                 '{ \"secret\": \"retained-value\" }', 'original-bindings',
                 '{ \"runner_image\": [\"original-image\"] }', 'Active', NULL, 1)",
            vec![key.into(), digest.into(), engine.into(), platform.into()],
        ))
        .await?;
    Ok(())
}

async fn seed_pins(store: &Store, digest: &str) -> TestResult {
    store
        .connection()
        .execute_unprepared(
            "INSERT INTO fleets (key, incarnation, desired_revision, observed_revision,
         mutation_fence, deletion_marker, phase, tombstone, created_at, updated_at)
         VALUES ('fleet', 'fleet-incarnation', 1, 1, 4, 0, 'Ready', 0, 1, 2);
         INSERT INTO profile_changes (id, resource_kind, profile_key, revision,
         kind, state, attempts, created_at, updated_at)
         VALUES ('change', 'template_profile', 'unique', 1, 'Publish', 'Converged', 1, 1, 2);",
        )
        .await?;
    store
        .connection()
        .execute(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "INSERT INTO fleet_revisions (fleet_key, incarnation, revision, spec_json,
         template_profile_key, template_revision, template_artifact_digest, template_attestation_id,
         auth_desired_profile_key, auth_desired_revision, inputs_digest, actor, created_at)
         VALUES ('fleet', 'fleet-incarnation', 1, '{\"original\":true}',
         'unique', 1, ?, 'attestation', 'auth', 1, 'input-digest', 'operator', 1)",
            vec![digest.into()],
        ))
        .await?;
    store
        .connection()
        .execute(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "INSERT INTO runner_generations (id, fleet_key, runner_name, generation_name,
         fleet_revision, template_profile_key, template_revision, template_artifact_digest,
         attestation_id, inputs_digest, state, workspace_path, created_at, updated_at)
         VALUES ('generation', 'fleet', 'runner', 'generation', 1, 'unique', 1, ?,
         'attestation', 'input-digest', 'Running', 'original/workspace', 1, 2)",
            vec![digest.into()],
        ))
        .await?;
    Ok(())
}

pub(super) async fn retained_snapshot(store: &Store) -> TestResult<BTreeMap<String, Vec<String>>> {
    let mut snapshot = BTreeMap::new();
    for table in [
        "template_profiles",
        "template_profile_revisions",
        "template_conformance_attestations",
        "template_artifacts",
        "profile_changes",
        "fleets",
        "fleet_revisions",
        "runner_generations",
        "artifact_archives",
    ] {
        let rows = store
            .connection()
            .query_all(Statement::from_string(
                DatabaseBackend::Sqlite,
                format!("PRAGMA table_info({table})"),
            ))
            .await?;
        let mut columns = Vec::new();
        for row in rows {
            let name: String = row.try_get("", "name")?;
            if name != "source_key" {
                columns.push(format!("quote(\"{}\")", name.replace('"', "\"\"")));
            }
        }
        let rows = store
            .connection()
            .query_all(Statement::from_string(
                DatabaseBackend::Sqlite,
                format!(
                    "SELECT json_array({}) AS value FROM {table} ORDER BY rowid",
                    columns.join(",")
                ),
            ))
            .await?;
        snapshot.insert(
            table.to_owned(),
            rows.into_iter()
                .map(|row| row.try_get("", "value"))
                .collect::<Result<Vec<String>, _>>()?,
        );
    }
    Ok(snapshot)
}

pub(super) async fn scalar(store: &Store, sql: &str) -> TestResult<i64> {
    let row = store
        .connection()
        .query_one(Statement::from_string(DatabaseBackend::Sqlite, sql))
        .await?
        .ok_or("missing scalar result")?;
    Ok(row.try_get("", "value")?)
}

pub(super) async fn restored(path: &Path) -> TestResult<Store> {
    let store = Store::open(path).await?;
    store.migrate().await?;
    Ok(store)
}
