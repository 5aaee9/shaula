use super::*;
use shaula_core::{
    state_backend::StateClaim,
    worker::{CompletionKind, CompletionReceipt},
};
type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

#[test]
fn cleanup_inventory_bounds_flat_and_nested_entries() -> TestResult {
    let root = tempfile::tempdir()?;
    for index in 0..8192 {
        std::fs::write(root.path().join(index.to_string()), b"")?;
    }
    let collect = || {
        let mut files = BTreeMap::new();
        let mut dirs = Vec::new();
        let result = inventory(root.path(), Path::new(""), &mut files, &mut dirs);
        (result, files.len() + dirs.len())
    };
    assert!(collect().0.is_ok(), "exact budget remains supported");
    let extra = root.path().join("extra");
    std::fs::write(&extra, b"")?;
    let (result, count) = collect();
    assert!(matches!(result, Err(StateError::TooLarge)));
    assert!(count <= 8192);
    std::fs::remove_file(extra)?;
    std::fs::remove_file(root.path().join("0"))?;
    std::fs::create_dir(root.path().join("nested"))?;
    std::fs::write(root.path().join("nested/extra"), b"")?;
    let (result, count) = collect();
    assert!(matches!(result, Err(StateError::TooLarge)));
    assert!(
        count <= 8192,
        "directories reserve budget before descending"
    );
    Ok(())
}

struct Fixture {
    root: tempfile::TempDir,
    request: WorkspaceCleanup,
}
impl Fixture {
    fn new() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let root = tempfile::tempdir()?;
        let id = uuid::Uuid::new_v4();
        let workspace = root.path().join("work").join(id.to_string());
        let artifact = root.path().join("artifacts/template");
        std::fs::create_dir_all(&workspace)?;
        std::fs::create_dir_all(&artifact)?;
        let mut archive = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::default(),
        ));
        for name in ["main.tf", "profile.yaml", ".terraform.lock.hcl"] {
            std::fs::write(artifact.join(name), b"original")?;
            std::fs::write(workspace.join(name), b"original")?;
            archive.append_path_with_name(artifact.join(name), name)?;
        }
        let bytes = archive.into_inner()?.finish()?;
        std::fs::write(artifact.with_extension("tar.gz"), &bytes)?;
        std::fs::write(workspace.join("shaula.tfvars.json"), b"protected")?;
        Ok(Self {
            root,
            request: WorkspaceCleanup {
                receipt: CompletionReceipt {
                    claim: StateClaim {
                        generation_id: id,
                        worker_attempt: uuid::Uuid::new_v4(),
                        worker_epoch: 1,
                    },
                    request_id: uuid::Uuid::new_v4(),
                    state_revision: 3,
                    kind: CompletionKind::ProviderCleanup,
                    completed_at: 10,
                },
                workspace,
                artifact,
                artifact_digest: format!("sha256:{}", hex::encode(Sha256::digest(&bytes))),
                retained_input_digest: Some(format!(
                    "sha256:{}",
                    hex::encode(Sha256::digest(b"protected"))
                )),
            },
        })
    }
    fn reap(&self) -> StateResult<CleanupOutcome> {
        reap(
            &self.root.path().join("work"),
            &self.root.path().join("artifacts"),
            &self.request,
        )
    }
}

#[test]
fn cleanup_preserves_emergency_unknown_and_unretained_material() -> TestResult {
    let f = Fixture::new()?;
    for name in [
        "errored.tfstate",
        "terraform.tfstate",
        "terraform.tfstate.backup",
        "unknown.txt",
    ] {
        let path = f.request.workspace.join(name);
        std::fs::write(&path, b"evidence")?;
        assert!(matches!(f.reap()?, CleanupOutcome::Retained));
        assert_eq!(
            std::fs::read(f.request.workspace.join("main.tf"))?,
            b"original"
        );
        assert_eq!(std::fs::read(&path)?, b"evidence");
        std::fs::remove_file(path)?;
    }
    std::fs::write(
        f.request.workspace.join("shaula.tfvars.json"),
        b"not-retained",
    )?;
    assert!(matches!(f.reap()?, CleanupOutcome::Retained));
    std::fs::write(f.request.workspace.join("shaula.tfvars.json"), b"protected")?;
    assert!(matches!(f.reap()?, CleanupOutcome::Reaped));
    assert!(!f.request.workspace.exists());
    assert!(f.request.artifact.with_extension("tar.gz").is_file());
    assert!(matches!(f.reap()?, CleanupOutcome::Reaped));
    Ok(())
}

#[test]
fn cleanup_receipt_cannot_authorize_another_generation_directory() -> TestResult {
    let mut f = Fixture::new()?;
    f.request.receipt.claim.generation_id = uuid::Uuid::new_v4();
    assert!(matches!(f.reap()?, CleanupOutcome::Retained));
    assert!(f.request.workspace.join("shaula.tfvars.json").is_file());
    Ok(())
}

#[test]
fn never_started_cleanup_handles_materialized_and_absent_input() -> TestResult {
    for materialized in [true, false] {
        let mut f = Fixture::new()?;
        f.request.receipt.kind = CompletionKind::NeverStarted;
        if !materialized {
            std::fs::remove_file(f.request.workspace.join("shaula.tfvars.json"))?;
            f.request.retained_input_digest = None;
        }
        assert!(matches!(f.reap()?, CleanupOutcome::Reaped));
        assert!(!f.request.workspace.exists());
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn cleanup_does_not_follow_redirected_paths() -> TestResult {
    let f = Fixture::new()?;
    let outside = f.root.path().join("evidence");
    std::fs::create_dir(&outside)?;
    std::fs::write(outside.join("keep"), b"evidence")?;
    std::os::unix::fs::symlink(&outside, f.request.workspace.join("redirect"))?;
    assert!(matches!(f.reap()?, CleanupOutcome::Retained));
    assert_eq!(std::fs::read(outside.join("keep"))?, b"evidence");
    Ok(())
}
