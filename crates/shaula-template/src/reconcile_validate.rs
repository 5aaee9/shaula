//! Static read-only validation of an artifact's optional `reconcile` module
//! (ARD-0044). Rejects any mutating or nested module block before execution.

use std::path::Path;

use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::template::ProfileManifest;

const MAX_BYTES: usize = 1024 * 1024;
const MAX_FILES: usize = 32;

const ALLOWED_BLOCKS: &[&str] = &[
    "terraform",
    "provider",
    "variable",
    "locals",
    "data",
    "output",
];

fn invalid(msg: impl Into<String>) -> CoreError {
    CoreError::new(ReasonCode::TemplateInvalid, msg)
}

/// Validates that `manifest.reconcile.discover_module` exists inside
/// `artifact_dir`, contains only regular `.tf` files with allowed read-only
/// HCL blocks, and declares both `variable "shaula"` and
/// `output "shaula_discover"`.
pub fn verify_reconcile_module(artifact_dir: &Path, manifest: &ProfileManifest) -> CoreResult<()> {
    let Some(reconcile) = &manifest.reconcile else {
        return Ok(());
    };
    reconcile.validate()?;
    let module_dir = artifact_dir.join(&reconcile.discover_module);
    let meta = std::fs::symlink_metadata(&module_dir)
        .map_err(|_| invalid("reconcile discover_module directory missing"))?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(invalid(
            "reconcile discover_module must be a regular directory",
        ));
    }

    let mut paths = Vec::new();
    for entry in std::fs::read_dir(&module_dir)
        .map_err(|_| invalid("reconcile discover_module unreadable"))?
    {
        let entry = entry.map_err(|_| invalid("reconcile discover_module unreadable"))?;
        let file_type = entry
            .file_type()
            .map_err(|_| invalid("reconcile discover_module entry unreadable"))?;
        if file_type.is_symlink() || file_type.is_dir() {
            return Err(invalid(
                "reconcile discover_module must not contain subdirectories or symlinks",
            ));
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let lower = name.to_lowercase();
        if lower.ends_with(".exe")
            || lower.ends_with(".sh")
            || lower.ends_with(".cmd")
            || lower.ends_with(".bat")
        {
            return Err(invalid("executable entry inside reconcile discover_module"));
        }
        if lower.ends_with(".tf.json") || lower == "override.tf" || lower.ends_with("_override.tf")
        {
            return Err(invalid(
                "JSON and override Terraform files are forbidden in reconcile discover_module",
            ));
        }
        if !name.ends_with(".tf") {
            return Err(invalid(
                "reconcile discover_module may contain only .tf files",
            ));
        }
        paths.push(entry.path());
        if paths.len() > MAX_FILES {
            return Err(invalid("reconcile discover_module exceeds file limit"));
        }
    }
    if paths.is_empty() {
        return Err(invalid(
            "reconcile discover_module must contain at least one .tf file",
        ));
    }
    paths.sort_unstable();

    let mut total_bytes = 0usize;
    let mut shaula_vars = 0usize;
    let mut discover_outputs = 0usize;
    for path in paths {
        let file = std::fs::File::open(&path)
            .map_err(|_| invalid("reconcile discover_module source unreadable"))?;
        let bytes = crate::artifact::read_bounded(file, MAX_BYTES as u64)?;
        total_bytes = total_bytes.saturating_add(bytes.len());
        if total_bytes > MAX_BYTES {
            return Err(invalid("reconcile discover_module exceeds 1 MiB limit"));
        }
        let text = String::from_utf8(bytes)
            .map_err(|_| invalid("reconcile discover_module source is not UTF-8"))?;
        crate::variables::guard::check(&text)?;
        let parsed: hcl::edit::structure::Body = text
            .parse()
            .map_err(|_| invalid("reconcile discover_module contains invalid HCL"))?;
        let normalize = |value: &str| value.replace("\r\n", "\n").replace('\r', "\n");
        if normalize(&parsed.to_string()) != normalize(&text) {
            return Err(invalid(
                "reconcile discover_module contains ambiguous or lossy object declarations",
            ));
        }
        for block in parsed.into_blocks() {
            let ident = block.ident.as_str();
            if !ALLOWED_BLOCKS.contains(&ident) {
                return Err(invalid(format!(
                    "reconcile discover_module forbids '{ident}' blocks; only read-only data/output declarations are allowed"
                )));
            }
            if ident == "variable"
                && block.labels.len() == 1
                && block.labels[0].as_str() == "shaula"
            {
                shaula_vars += 1;
            }
            if ident == "output"
                && block.labels.len() == 1
                && block.labels[0].as_str() == "shaula_discover"
            {
                discover_outputs += 1;
            }
        }
    }
    if shaula_vars != 1 {
        return Err(invalid(
            "reconcile discover_module must declare exactly one variable \"shaula\"",
        ));
    }
    if discover_outputs != 1 {
        return Err(invalid(
            "reconcile discover_module must declare exactly one output \"shaula_discover\"",
        ));
    }
    Ok(())
}
