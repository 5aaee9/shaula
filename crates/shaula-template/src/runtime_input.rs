use super::digest_of;
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use std::path::Path;

/// Writes the protected input tfvars with bounded, durable semantics
/// (R9-08, spec 0001 §10.2 / 0004 §9): the file is created in the same
/// directory with restrictive permissions from creation, flushed and
/// fsynced, then atomically published without replacement so a crash cannot leave a
/// half-written Destroy input behind.
pub(super) fn write_protected_input(
    workspace: &Path,
    input: &shaula_core::template::ShaulaInputEnvelope,
) -> CoreResult<String> {
    use std::io::Write;
    let content = input.to_tfvars()?;
    let path = workspace.join("shaula.tfvars.json");
    if path.exists() {
        let previous = std::fs::read(&path).map_err(|_| state_err_core())?;
        if previous == content.as_bytes() {
            return Ok(digest_of(content.as_bytes()));
        }
        return Err(state_err_core());
    }
    // NamedTempFile creates mode 0600 on Unix before any secret is written.
    let mut file = tempfile::NamedTempFile::new_in(workspace).map_err(|e| {
        CoreError::new(
            ReasonCode::Internal,
            format!("cannot write protected input: {e}"),
        )
    })?;
    file.write_all(content.as_bytes())
        .and_then(|()| file.as_file().sync_all())
        .map_err(|e| {
            CoreError::new(
                ReasonCode::Internal,
                format!("cannot persist protected input: {e}"),
            )
        })?;
    if let Err(error) = file.persist_noclobber(&path) {
        if error.error.kind() != std::io::ErrorKind::AlreadyExists
            || std::fs::read(&path).map_err(|_| state_err_core())? != content.as_bytes()
        {
            return Err(state_err_core());
        }
    }
    // fsync the directory so the rename itself survives power loss.
    #[cfg(unix)]
    if let Ok(dir) = std::fs::File::open(workspace) {
        let _ = dir.sync_all();
    }
    Ok(digest_of(content.as_bytes()))
}

fn state_err_core() -> CoreError {
    CoreError::new(
        ReasonCode::TemplateInvalid,
        "protected input cannot be replaced or recovered",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use shaula_core::template::{BindingsDigest, GenerationIdentity, ShaulaInputEnvelope};

    #[test]
    fn frozen_input_is_idempotent_and_cannot_be_replaced() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir = tempfile::tempdir()?;
        let mut input = ShaulaInputEnvelope::new(
            GenerationIdentity {
                fleet_key: "f".into(),
                scale_set_id: 1,
                id: "g".into(),
                runner_name: "r".into(),
                generation_name: "generation".into(),
            },
            "private-jit".into(),
            BindingsDigest("commitment".into()),
        );
        let digest = write_protected_input(dir.path(), &input)?;
        let path = dir.path().join("shaula.tfvars.json");
        let original = std::fs::read(&path)?;
        assert_eq!(write_protected_input(dir.path(), &input)?, digest);
        input.jit_config = "different-jit".into();
        assert!(write_protected_input(dir.path(), &input).is_err());
        assert_eq!(std::fs::read(&path)?, original);
        assert_eq!(std::fs::read_dir(dir.path())?.count(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(path)?.permissions().mode() & 0o777, 0o600);
        }
        Ok(())
    }
}
