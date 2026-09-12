//! Host-owned, one-shot launch after the admitted Create apply completes.
//! Payloads never execute in the Runner container and no Create is retried here.

use super::TemplateRuntime;
use shaula_core::{
    operation_log::SetupProjection,
    ports::{TemplateCreateRequest, TemplateOutcomeError},
    template::{ProfileManifest, ResultResource, ShaulaResultEnvelope},
};
use std::{
    io::Write,
    path::Path,
    time::{Duration, Instant},
};

#[path = "runtime_bootstrap_docker.rs"]
mod docker;
#[path = "runtime_bootstrap_kubernetes.rs"]
mod kubernetes;
#[path = "runtime_bootstrap_plan.rs"]
mod plan;
#[path = "runtime_bootstrap_process.rs"]
mod process;
pub(super) use plan::admit_plan;
use process::Commands;

const GROUP: &str = "Terraform apply (runner provisioning)";
const MAX_DETAIL_BYTES: usize = 512 * 1024;
const MAX_BOOTSTRAP: Duration = Duration::from_secs(60);
const LISTENER: &str = "/home/runner/bin/Runner.Listener";

fn failed() -> TemplateOutcomeError {
    TemplateOutcomeError::ExecutionFailed {
        phase: "container.bootstrap".into(),
    }
}

/// The caller revalidates the frozen manifest and holds bootstrap admission.
/// Diagnostic retrieval is optional; identity and launch checks fail closed.
pub(super) async fn launch(
    runtime: &TemplateRuntime,
    request: &TemplateCreateRequest,
    envelope: &ShaulaResultEnvelope,
) -> Result<(), TemplateOutcomeError> {
    let started = Instant::now();
    let manifest = std::fs::read_to_string(request.artifact_dir.join("profile.yaml"))
        .ok()
        .and_then(|text| crate::manifest::parse_manifest(&text).ok())
        .ok_or_else(failed)?;
    let image = selected_image(&manifest, request)?;
    // Crash residue must not alter the immutable workspace material digest.
    let temporary = private_directory().ok();
    let projection = match &runtime.operation_log_reader {
        Some(reader) => tokio::time::timeout(
            Duration::from_secs(5),
            reader.setup_projection(&request.input.generation.id),
        )
        .await
        .ok()
        .and_then(Result::ok),
        None => None,
    };
    let setup = render(projection);
    let timeout = request
        .timeout
        .min(MAX_BOOTSTRAP)
        .saturating_sub(started.elapsed());
    if timeout.is_zero() {
        return Err(failed());
    }
    match manifest.platform.as_str() {
        "docker" => {
            docker::launch(
                request,
                envelope,
                &image,
                manifest.runner_backend.as_str(),
                &setup,
                temporary.as_ref().map(|dir| dir.path()),
                timeout,
            )
            .await
        }
        "kubernetes" => {
            kubernetes::launch(
                request,
                envelope,
                &image,
                manifest.runner_backend.as_str(),
                &setup,
                temporary.as_ref().ok_or_else(failed)?.path(),
                timeout,
            )
            .await
        }
        _ => Err(failed()),
    }
}

fn selected_image(
    manifest: &ProfileManifest,
    request: &TemplateCreateRequest,
) -> Result<String, TemplateOutcomeError> {
    let requested = request.input.parameters.get("runner_image");
    let image = match requested {
        Some(serde_json::Value::String(alias)) => manifest
            .runner_image_digests
            .iter()
            .find(|image| image.split('@').next() == Some(alias)),
        None if manifest.runner_image_digests.len() == 1 => manifest.runner_image_digests.first(),
        _ => None,
    }
    .ok_or_else(failed)?;
    let (repository, digest) = image.split_once("@sha256:").ok_or_else(failed)?;
    let official = match manifest.runner_backend.as_str() {
        "github" => {
            repository == "ghcr.io/actions/actions-runner"
                || repository
                    .strip_prefix("ghcr.io/actions/actions-runner:")
                    .is_some_and(|tag| !tag.is_empty() && !tag.contains('/'))
        }
        "forgejo" => {
            repository == "code.forgejo.org/forgejo/runner"
                || repository
                    .strip_prefix("code.forgejo.org/forgejo/runner:")
                    .is_some_and(|tag| !tag.is_empty() && !tag.contains('/'))
        }
        _ => false,
    };
    if !official {
        return Err(failed());
    }
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(failed());
    }
    Ok(image.clone())
}

fn resource<'a>(
    envelope: &'a ShaulaResultEnvelope,
    role: &str,
) -> Result<&'a ResultResource, TemplateOutcomeError> {
    let mut matches = envelope.resources.iter().filter(|item| item.role == role);
    let item = matches.next().ok_or_else(failed)?;
    if matches.next().is_some() {
        return Err(failed());
    }
    Ok(item)
}

fn binding<'a>(
    request: &'a TemplateCreateRequest,
    name: &str,
) -> Result<&'a str, TemplateOutcomeError> {
    request
        .input
        .bindings
        .get(name)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty() && !value.contains('\0'))
        .ok_or_else(failed)
}

fn render(projection: Option<SetupProjection>) -> Vec<u8> {
    let approved = projection.filter(|projection| projection.status == "ready");
    let partial = approved
        .as_ref()
        .is_some_and(|projection| projection.partial);
    let mut detail = approved
        .and_then(|projection| projection.detail)
        .unwrap_or_else(|| {
            "[Shaula: setup info unavailable or withheld; inspect authorized operation logs]".into()
        });
    if partial && !detail.contains("incomplete or truncated") {
        detail.push_str("\n[Shaula: provisioning log is incomplete or truncated]\n");
    }
    let detail = bounded_detail(&detail);
    let bytes = serde_json::to_vec(&serde_json::json!([{"Group": GROUP, "Detail": detail}]))
        .unwrap_or_else(|_| b"[]".to_vec());
    // Keep the projection bounded after JSON escaping. The Kubernetes adapter
    // additionally budgets its Secret against the actual frozen JIT length.
    if bytes.len() <= 768 * 1024 {
        bytes
    } else {
        serde_json::to_vec(&serde_json::json!([
            {"Group": GROUP, "Detail": truncated_detail(&detail)}
        ]))
        .unwrap_or_else(|_| b"[]".to_vec())
    }
}

fn bounded_detail(detail: &str) -> String {
    if detail.len() <= MAX_DETAIL_BYTES && detail.lines().count() <= 20_000 {
        return detail.into();
    }
    truncated_detail(detail)
}

fn truncated_detail(detail: &str) -> String {
    let head: String = detail
        .lines()
        .take(5000)
        .collect::<Vec<_>>()
        .join("\n")
        .chars()
        .take(32_768)
        .collect();
    let tail_lines = detail.lines().rev().take(5000).collect::<Vec<_>>();
    let tail = tail_lines.into_iter().rev().collect::<Vec<_>>().join("\n");
    let tail: String = tail.chars().rev().take(32_768).collect();
    let tail: String = tail.chars().rev().collect();
    format!("{head}\n[Shaula: setup info truncated; full approved log in Web UI]\n{tail}")
}

fn private_directory() -> std::io::Result<tempfile::TempDir> {
    let mut builder = tempfile::Builder::new();
    builder.prefix("shaula-bootstrap-");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Apply permissions in mkdir, before any payload can be created.
        builder.permissions(std::fs::Permissions::from_mode(0o700));
    }
    builder.tempdir()
}

fn protected_file(
    directory: &Path,
    bytes: &[u8],
    runner_readable: bool,
) -> Result<tempfile::NamedTempFile, TemplateOutcomeError> {
    let mut file = tempfile::NamedTempFile::new_in(directory).map_err(|_| failed())?;
    file.write_all(bytes)
        .and_then(|()| file.as_file().sync_all())
        .map_err(|_| failed())?;
    // docker cp preserves mode and defaults destination ownership to root.
    // Docker copies require the non-root runner to read root-owned files.
    // Mode 0444 stays behind a host-private (0700) directory and is exposed
    // only inside this Generation's execution domain, never a host bind mount.
    // Kubernetes patch files are not copied and remain mode 0600.
    #[cfg(unix)]
    if runner_readable {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o444))
            .map_err(|_| failed())?;
    }
    #[cfg(not(unix))]
    let _ = runner_readable;
    Ok(file)
}

#[cfg(test)]
#[path = "runtime_bootstrap_tests.rs"]
mod tests;
