//! Workspace layout: per-generation directories with containment checks.
//! A workspace is exclusively owned by one Runner Generation.

use std::path::{Path, PathBuf};

use shaula_core::error::{CoreError, CoreResult, ReasonCode};

fn err(msg: impl Into<String>) -> CoreError {
    CoreError::new(ReasonCode::Internal, msg)
}

/// Resolves and validates the workspace path for one generation under the
/// configured work root. Rejects traversal and workspace aliasing.
pub fn generation_workspace(
    work_root: &Path,
    fleet_key: &str,
    generation_id: &str,
) -> CoreResult<PathBuf> {
    validate_segment(fleet_key)?;
    validate_segment(generation_id)?;
    Ok(work_root.join(fleet_key).join(generation_id))
}

/// Ensures a path stays inside its declared root after normalization.
pub fn ensure_contained(root: &Path, candidate: &Path) -> CoreResult<()> {
    let root_abs = root
        .canonicalize()
        .map_err(|e| err(format!("workspace root unreadable: {e}")))?;
    let candidate_abs = candidate
        .canonicalize()
        .map_err(|e| err(format!("workspace path unreadable: {e}")))?;
    if !candidate_abs.starts_with(&root_abs) {
        return Err(err("workspace escapes configured root"));
    }
    Ok(())
}

fn validate_segment(segment: &str) -> CoreResult<()> {
    if segment.is_empty()
        || segment.len() > 128
        || segment.contains("..")
        || segment.contains('/')
        || segment.contains('\\')
        || segment.contains(':')
    {
        return Err(err("workspace path segment invalid"));
    }
    Ok(())
}

/// Creates the workspace directory tree for a generation.
pub fn create_workspace(path: &Path) -> CoreResult<()> {
    std::fs::create_dir_all(path).map_err(|e| err(format!("cannot create workspace: {e}")))
}

/// Removes a workspace recursively; only called after terminal Destroy.
pub fn remove_workspace(path: &Path) -> CoreResult<()> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(err(format!("cannot remove workspace: {e}"))),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn workspace_paths_are_namespaced() {
        let root = Path::new("/work-root");
        let path = generation_workspace(root, "fleet-a", "gen-1").unwrap();
        let rendered = path.to_string_lossy().replace('\\', "/");
        assert_eq!(rendered, "/work-root/fleet-a/gen-1");
    }

    #[test]
    fn traversal_segments_rejected() {
        let root = Path::new("/work-root");
        assert!(generation_workspace(root, "..", "gen").is_err());
        assert!(generation_workspace(root, "fleet", "a/b").is_err());
        assert!(generation_workspace(root, "fleet", "C:evil").is_err());
        assert!(generation_workspace(root, "", "gen").is_err());
    }

    #[test]
    fn containment_check_rejects_escape() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        let inside = root.join("gen");
        std::fs::create_dir_all(&inside).unwrap();
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();

        ensure_contained(&root, &inside).unwrap();
        assert!(ensure_contained(&root, &outside).is_err());
    }
}

/// Read-only state identity pulled from the workspace backend: managed
/// resource addresses (deletion targets), serial and lineage. The
/// lineage is REQUIRED — a state without a lineage cannot prove
/// ownership and is refused at parse time (spec 0004 §5).
#[derive(Debug, Clone)]
pub struct StateSnapshot {
    pub managed: Vec<String>,
    pub serial: u64,
    pub lineage: String,
}

/// Engine-generated files/directories are NOT template material: they
/// appear only after init/plan/apply and must never participate in the
/// workspace digest. The exclusion list is EXACT (R5-07): a prefix match
/// would swallow legitimate template source like
/// `terraform.tfstate_extra.tf`, which is ordinary executable `.tf`
/// material.
fn engine_generated(rel: &std::path::Path) -> bool {
    let mut components = rel.components();
    if components.any(|c| c.as_os_str() == ".terraform" || c.as_os_str() == "terraform.tfstate.d") {
        return true;
    }
    let Some(name) = rel.file_name().and_then(|n| n.to_str()) else {
        return true;
    };
    matches!(
        name,
        "tfplan"
            | "shaula.tfvars.json"
            | "terraform.tfstate"
            | "terraform.tfstate.backup"
            | ".terraform.tfstate.lock.info"
    )
}

/// Digest over the WHOLE executable template material of a directory —
/// every file, recursively, in sorted relative-path order. Generated
/// artifacts like `tfplan`, `shaula.tfvars.json`, `.terraform/` or
/// `terraform.tfstate*` are excluded, so two directories holding the
/// same template material compare equal regardless of engine
/// bookkeeping — while an ADDED module or variables file changes the
/// pin (spec 0004 §5: the provenance binds the full executed material).
pub fn template_files_digest(dir: &Path) -> CoreResult<String> {
    material_digest(dir, false)
}

/// Explicit HTTP-runtime digest: only the verified fixed root backend file is
/// outside the immutable artifact. Legacy callers get no such exception.
pub(crate) fn http_template_files_digest(dir: &Path) -> CoreResult<String> {
    crate::http_backend::verify_system_file(dir)?;
    material_digest(dir, true)
}

fn material_digest(dir: &Path, http_backend: bool) -> CoreResult<String> {
    for name in ["main.tf", "profile.yaml", ".terraform.lock.hcl"] {
        if !dir.join(name).is_file() {
            return Err(err(format!(
                "template file {} missing",
                dir.join(name).display()
            )));
        }
    }
    fn collect(
        root: &Path,
        dir: &Path,
        into: &mut Vec<PathBuf>,
        http_backend: bool,
    ) -> CoreResult<()> {
        let entries = std::fs::read_dir(dir)
            .map_err(|e| err(format!("template dir {} unreadable: {e}", dir.display())))?;
        for entry in entries {
            let entry = entry.map_err(|e| err(format!("template dir entry unreadable: {e}")))?;
            let path = entry.path();
            let rel = path
                .strip_prefix(root)
                .map_err(|_| err("template path escaped its root"))?;
            if engine_generated(rel)
                || (http_backend && rel == Path::new(crate::http_backend::BACKEND_FILE))
            {
                continue;
            }
            if path.is_dir() {
                collect(root, &path, into, http_backend)?;
            } else {
                into.push(rel.to_path_buf());
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    collect(dir, dir, &mut files, http_backend)?;
    files.sort();
    use sha2::{Digest as _, Sha256};
    let mut hasher = Sha256::new();
    for rel in &files {
        // Forward-slash canonical form so Windows and Unix agree.
        let canonical: String = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        let bytes = std::fs::read(dir.join(rel))
            .map_err(|e| err(format!("template file {} unreadable: {e}", rel.display())))?;
        hasher.update(canonical.as_bytes());
        hasher.update([0]);
        hasher.update(&bytes);
    }
    Ok(format!("sha256:{}", hex::encode(hasher.finalize())))
}

#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#[cfg(test)]
mod digest_tests {
    use super::*;

    fn materialize(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("main.tf"), b"resource \"x\" \"y\" {}\n").unwrap();
        std::fs::write(dir.join("profile.yaml"), b"api_version: x\n").unwrap();
        std::fs::write(dir.join(".terraform.lock.hcl"), b"# lock\n").unwrap();
    }

    #[test]
    fn extra_template_files_change_the_digest() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        materialize(dir);
        let before = template_files_digest(dir).unwrap();
        std::fs::write(dir.join("variables.tf"), b"variable \"a\" {}\n").unwrap();
        let after = template_files_digest(dir).unwrap();
        assert_ne!(before, after, "added executable material must re-pin");
        // Local modules are material too.
        let modules = dir.join("modules").join("runner");
        std::fs::create_dir_all(&modules).unwrap();
        std::fs::write(modules.join("main.tf"), b"resource \"m\" \"n\" {}\n").unwrap();
        assert_ne!(after, template_files_digest(dir).unwrap());
    }

    #[test]
    fn engine_generated_files_do_not_change_the_digest() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        materialize(dir);
        let before = template_files_digest(dir).unwrap();
        std::fs::create_dir_all(dir.join(".terraform").join("providers")).unwrap();
        std::fs::write(dir.join(".terraform").join("terraform.tfstate"), b"{}").unwrap();
        std::fs::write(dir.join("terraform.tfstate"), b"{\"version\":4}").unwrap();
        std::fs::write(dir.join("terraform.tfstate.backup"), b"{\"version\":4}").unwrap();
        std::fs::write(dir.join("tfplan"), b"plan-bytes").unwrap();
        std::fs::write(dir.join("shaula.tfvars.json"), b"{\"jit\":\"s\"}").unwrap();
        let after = template_files_digest(dir).unwrap();
        assert_eq!(
            before, after,
            "engine bookkeeping must stay outside the pin"
        );
    }

    #[test]
    fn tfstate_prefix_is_not_an_exclusion_wildcard() {
        // R5-07: `terraform.tfstate_extra.tf` is ordinary template
        // source — a `starts_with("terraform.tfstate")` rule hid it from
        // the pin.
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        materialize(dir);
        let before = template_files_digest(dir).unwrap();
        std::fs::write(
            dir.join("terraform.tfstate_extra.tf"),
            b"resource \"null_resource\" \"extra\" {}\n",
        )
        .unwrap();
        let after = template_files_digest(dir).unwrap();
        assert_ne!(before, after, "state-prefixed .tf source is still material");
    }

    #[test]
    fn missing_required_material_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(template_files_digest(tmp.path()).is_err());
    }
}
