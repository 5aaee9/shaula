use std::path::{Path, PathBuf};

use sea_orm::{sea_query::Expr, EntityTrait};

use crate::entities::{shared::audit_records, template::template_profile_revisions};
use crate::registry_impl::SqliteControlPlane;

use super::template_activation_support::{Fixture, TestResult};

fn materialize(fixture: &Fixture) -> TestResult<(SqliteControlPlane, PathBuf)> {
    let root = fixture.temp.path().join("artifacts");
    let digest = format!("sha256:{}", "a".repeat(64));
    let material =
        shaula_core::artifact_layout::artifact_dir(&root, &digest).ok_or("invalid test digest")?;
    std::fs::create_dir_all(material.join("schemas"))?;
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/docker/profile.yaml"),
        material.join("profile.yaml"),
    )?;
    std::fs::write(material.join("schemas/parameters.schema.json"), "{}")?;
    std::fs::write(
        material.join(".terraform.lock.hcl"),
        "# provider-free fixture\n",
    )?;
    Ok((
        SqliteControlPlane::new(fixture.store.clone(), root),
        material,
    ))
}

#[tokio::test]
async fn invalid_dependency_lock_is_rejected_with_a_bounded_reason() -> TestResult {
    let fixture = Fixture::new().await?;
    let (scan, material) = materialize(&fixture)?;
    std::fs::write(
        material.join(".terraform.lock.hcl"),
        "secret-token=raw-sensitive-example",
    )?;
    let result = scan.periodic_scan(4).await?;
    assert_eq!(result.candidates_rejected, 1);
    assert_eq!(result.candidates_activated, 0);
    let (profile, candidate) = fixture.snapshot().await?;
    assert_eq!(profile.active_revision, None);
    assert_eq!(profile.status, "Rejected");
    assert_eq!(candidate.state, "Rejected");
    assert_eq!(candidate.reason.as_deref(), Some("DependencyLockInvalid"));
    let audits = audit_records::Entity::find()
        .all(fixture.store.connection())
        .await?;
    assert_eq!(audits.len(), 1);
    assert!(!audits[0]
        .detail_json
        .as_deref()
        .ok_or("audit missing")?
        .contains("raw-sensitive-example"));
    assert_eq!(scan.periodic_scan(5).await?.candidates_rejected, 0);
    Ok(())
}

#[tokio::test]
async fn unreadable_manifest_stays_ready_and_can_retry_after_recovery() -> TestResult {
    let fixture = Fixture::new().await?;
    let (scan, material) = materialize(&fixture)?;
    template_profile_revisions::Entity::update_many()
        .col_expr(
            template_profile_revisions::Column::State,
            Expr::value("Ready"),
        )
        .exec(fixture.store.connection())
        .await?;
    let before = fixture.snapshot().await?;
    let manifest = material.join("profile.yaml");
    let original = std::fs::read(&manifest)?;
    // A present but undecodable authority is an I/O/decode failure, not an
    // invalid parsed manifest that can receive a permanent rejection verdict.
    std::fs::write(&manifest, [0xff, 0xfe])?;
    let error = scan
        .periodic_scan(4)
        .await
        .err()
        .ok_or("scan should fail")?;
    assert_eq!(
        error.code,
        shaula_core::error::ReasonCode::StorageUnavailable
    );
    assert_eq!(fixture.snapshot().await?, before);
    assert!(audit_records::Entity::find()
        .all(fixture.store.connection())
        .await?
        .is_empty());
    std::fs::write(manifest, original)?;
    assert_eq!(scan.periodic_scan(5).await?.candidates_activated, 1);
    assert_eq!(fixture.snapshot().await?.0.active_revision, Some(1));
    Ok(())
}

#[tokio::test]
async fn absent_material_and_invalid_manifest_never_activate() -> TestResult {
    for reason in [
        "ArtifactNotPublished",
        "ArtifactShapeInvalid",
        "ManifestInvalid",
    ] {
        let fixture = Fixture::new().await?;
        let (scan, material) = materialize(&fixture)?;
        match reason {
            "ArtifactNotPublished" => std::fs::remove_file(material.join("profile.yaml"))?,
            "ArtifactShapeInvalid" => {
                std::fs::remove_file(material.join("schemas/parameters.schema.json"))?
            }
            _ => std::fs::write(
                material.join("profile.yaml"),
                "secret: never-disclose-this\n",
            )?,
        }
        assert_eq!(scan.periodic_scan(4).await?.candidates_rejected, 1);
        let (profile, candidate) = fixture.snapshot().await?;
        assert_eq!(profile.active_revision, None);
        assert_eq!(candidate.reason.as_deref(), Some(reason));
    }
    Ok(())
}
