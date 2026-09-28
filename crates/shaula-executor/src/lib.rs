//! Local exec process ownership. No Terraform commands or runner-backend policy.

mod containment;
mod process;

pub use process::ExecExecutor;

/// Offline legacy service fence: the operator supplies the original service's
/// containment root, never a source workspace path. It must still exist and be
/// empty, and its host/boot/path identity must match at inspect and apply.
pub fn legacy_fence(root: &std::path::Path) -> shaula_core::state_backend::StateResult<String> {
    let root = containment::root(root)?;
    if !containment::empty(&root)? {
        return Err(shaula_core::state_backend::StateError::Conflict);
    }
    Ok(format!("{}:{}", containment::boot()?, root.display()))
}
