use super::Entry;
use sha2::{Digest, Sha256};
use shaula_core::{
    ports::PlanProvenance,
    state_backend::{StateDocument, MAX_STATE_BYTES},
};
use shaula_daemon::bootstrap::ValidatedBootstrap;
use shaula_store::http_state::{LegacyGeneration, MigrationClassification};
use std::path::Path;

pub(super) struct Inspected {
    pub entry: Entry,
    pub state: Option<StateDocument>,
    pub input: Option<Vec<u8>>,
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

fn read(root: &Path, path: &Path, limit: usize) -> Result<Option<Vec<u8>>, String> {
    let metadata = match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        other => other.map_err(|_| "source file unreadable")?,
    };
    let canonical = std::fs::canonicalize(path).map_err(|_| "source file unreadable")?;
    let root = std::fs::canonicalize(root).map_err(|_| "source root unavailable")?;
    if !metadata.is_file() || metadata.len() > limit as u64 || !canonical.starts_with(root) {
        return Err("source file containment or size invalid".into());
    }
    let file = std::fs::File::open(path).map_err(|_| "source file unreadable")?;
    shaula_template::artifact::read_bounded(file, limit as u64)
        .map(Some)
        .map_err(|_| "source file exceeds bound or cannot be read".into())
}

pub(super) fn inspect(
    generation: &LegacyGeneration,
    config: &ValidatedBootstrap,
    engine: &str,
    fenced: bool,
) -> Result<Inspected, String> {
    let workspace = Path::new(&generation.workspace_path);
    let mut files = Vec::new();
    let mut bytes = Vec::new();
    for name in [
        "terraform.tfstate",
        "terraform.tfstate.backup",
        "errored.tfstate",
        "shaula.tfvars.json",
    ] {
        let result = read(&config.work_root, &workspace.join(name), MAX_STATE_BYTES)?;
        let commitment = match &result {
            Some(bytes) => digest(bytes),
            None => "missing".into(),
        };
        files.push((name, commitment));
        bytes.push(result);
    }
    let artifact = shaula_core::artifact_layout::artifact_dir(
        &config.artifact_root,
        &generation.template_artifact_digest,
    );
    let material = artifact
        .as_deref()
        .and_then(|path| shaula_template::workspace::template_files_digest(path).ok());
    // Verify the archive independently of its materialized copy.
    let archive = match artifact.as_deref() {
        Some(path) => read(
            &config.artifact_root,
            &path.with_extension("tar.gz"),
            256 * 1024 * 1024,
        )?
        .map(|bytes| digest(&bytes)),
        None => None,
    };
    let effects: serde_json::Value =
        serde_json::from_str(&generation.effects).map_err(|_| "operation history invalid")?;
    let creates: Vec<_> = effects
        .as_array()
        .ok_or("operation history is not an array")?
        .iter()
        .filter(|op| op.get("kind").and_then(|v| v.as_str()) == Some("Create"))
        .collect();
    // Multiple historical Create records require explicit reconciliation; an
    // arbitrary first record is not proof of the original material tuple.
    let original = (creates.len() == 1)
        .then(|| creates[0])
        .and_then(|op| op.get("provenance"))
        .and_then(|v| v.as_str())
        .and_then(|json| serde_json::from_str::<PlanProvenance>(json).ok());
    let state = bytes[0]
        .clone()
        .and_then(|bytes| StateDocument::parse(bytes).ok());
    let backup = bytes[1]
        .clone()
        .and_then(|bytes| StateDocument::parse(bytes).ok());
    let input = bytes[3].clone();
    let valid = fenced
        && bytes[2].is_none()
        && archive.as_deref() == Some(&generation.template_artifact_digest)
        && state.as_ref().is_some_and(|state| {
            generation.state_matches_original(state)
                && backup.as_ref().is_none_or(|backup| {
                    backup.lineage() == state.lineage() && backup.serial() <= state.serial()
                })
        })
        && (bytes[1].is_none() || backup.is_some())
        && original.as_ref().is_some_and(|proof| {
            proof.generation_id == generation.id
                && proof.engine_binary_digest == engine
                && proof.artifact_digest == generation.template_artifact_digest
                && material.as_deref() == Some(&proof.template_material_digest)
                && input
                    .as_ref()
                    .is_some_and(|input| digest(input) == proof.protected_input_digest)
        });
    let classification = if valid {
        MigrationClassification::Imported
    } else {
        MigrationClassification::Quarantined
    };
    let commitment = digest(
        &serde_json::to_vec(&serde_json::json!({
            "database": generation.commitment().map_err(|_| "generation commitment invalid")?,
            "files": files, "material": material, "archive": archive, "engine": engine,
            "classification": classification, "fenced": fenced,
        }))
        .map_err(|_| "commitment encoding failed")?,
    );
    Ok(Inspected {
        entry: Entry {
            id: generation.id.clone(),
            commitment,
            classification,
            reason: if valid {
                "exact state and original runtime verified"
            } else {
                "missing fence or incomplete original state/input/runtime evidence"
            }
            .into(),
        },
        state: if valid { state } else { None },
        input: if valid { input } else { None },
    })
}
