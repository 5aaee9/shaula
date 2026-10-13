//! GitLab `run-single` in the official image (spec 0043 §5). The token
//! reaches the runner only inside the copied `config.toml`.
use super::pool::{self, PoolLaunch};
use super::{failed, TemplateCreateRequest, TemplateOutcomeError};
use serde_json::{json, Value};
use shaula_core::gitlab::RUNNER_WAIT_TIMEOUT_SECS;
use shaula_core::ports::gitlab::GitlabBootstrapMaterial;
use std::path::Path;

use super::Commands;

pub(super) const CONFIG_PATH: &str = "/home/gitlab-runner/config.toml";

pub(super) async fn launch(
    commands: &Commands<'_>,
    id: &str,
    image_id: &str,
    request: &TemplateCreateRequest,
    temporary: Option<&Path>,
) -> Result<(), TemplateOutcomeError> {
    let material = material(request)?;
    let shape = shape(material);
    pool::launch(
        commands,
        id,
        image_id,
        request,
        temporary,
        &shape,
        &material.config_toml(),
    )
    .await
}

pub(super) fn validate(
    info: &Value,
    id: &str,
    image_id: &str,
    request: &TemplateCreateRequest,
) -> Result<(), TemplateOutcomeError> {
    let material = material(request)?;
    pool::validate(info, id, image_id, request, &shape(material))
}

fn material(
    request: &TemplateCreateRequest,
) -> Result<&GitlabBootstrapMaterial, TemplateOutcomeError> {
    let material = request.gitlab_bootstrap.as_ref().ok_or_else(failed)?;
    // The non-secret Terraform input must name the exact same registration.
    if request.input.gitlab.as_ref() != Some(&material.identity()) {
        return Err(failed());
    }
    Ok(material)
}

fn shape(material: &GitlabBootstrapMaterial) -> PoolLaunch<'_> {
    PoolLaunch {
        command: command(material),
        working_dir: "/home/gitlab-runner",
        user: Some("gitlab-runner"),
        volumes: &["/etc/gitlab-runner", "/home/gitlab-runner"],
        secret: material.token(),
        destination: CONFIG_PATH,
    }
}

pub(super) fn command(material: &GitlabBootstrapMaterial) -> Value {
    let mut command = vec!["/usr/bin/gitlab-runner".to_string()];
    command.extend(material.run_single_args(CONFIG_PATH, RUNNER_WAIT_TIMEOUT_SECS));
    json!(command)
}
