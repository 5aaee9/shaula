//! Workspace materialization from the content-addressed artifact,
//! split to keep files within 400 lines (AGENTS.md).

use std::path::PathBuf;

use shaula_core::error::{CoreError, CoreResult, ReasonCode};

pub(crate) fn copy_recursive(src: &PathBuf, dest: &PathBuf) -> CoreResult<()> {
    let meta = std::fs::metadata(src)
        .map_err(|e| CoreError::new(ReasonCode::Internal, format!("artifact unreadable: {e}")))?;
    if meta.is_dir() {
        std::fs::create_dir_all(dest).map_err(|e| {
            CoreError::new(
                ReasonCode::Internal,
                format!("cannot create workspace dir: {e}"),
            )
        })?;
        for entry in std::fs::read_dir(src).map_err(|e| {
            CoreError::new(
                ReasonCode::Internal,
                format!("artifact dir unreadable: {e}"),
            )
        })? {
            let entry = entry.map_err(|e| {
                CoreError::new(
                    ReasonCode::Internal,
                    format!("artifact entry unreadable: {e}"),
                )
            })?;
            copy_recursive(&entry.path(), &dest.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(src, dest).map_err(|e| {
            CoreError::new(
                ReasonCode::Internal,
                format!("cannot copy artifact file: {e}"),
            )
        })?;
        Ok(())
    }
}
