use super::pool::{self, PoolLaunch};
use super::{failed, TemplateCreateRequest, TemplateOutcomeError};
use serde_json::{json, Value};
use std::path::Path;

use super::Commands;

pub(super) async fn launch(
    commands: &Commands<'_>,
    id: &str,
    image_id: &str,
    request: &TemplateCreateRequest,
    temporary: Option<&Path>,
) -> Result<(), TemplateOutcomeError> {
    let shape = shape(request)?;
    pool::launch(
        commands,
        id,
        image_id,
        request,
        temporary,
        &shape,
        shape.secret,
    )
    .await
}

pub(super) fn validate(
    info: &Value,
    id: &str,
    image_id: &str,
    request: &TemplateCreateRequest,
) -> Result<(), TemplateOutcomeError> {
    pool::validate(info, id, image_id, request, &shape(request)?)
}

fn shape(request: &TemplateCreateRequest) -> Result<PoolLaunch<'_>, TemplateOutcomeError> {
    let material = request.forgejo_bootstrap.as_ref().ok_or_else(failed)?;
    if material.instance_url.contains('\0')
        || material.uuid.contains('\0')
        || material.token().contains('\0')
    {
        return Err(failed());
    }
    Ok(PoolLaunch {
        command: command(material),
        working_dir: "/data",
        user: None,
        volumes: &["/data"],
        secret: material.token(),
        destination: "/data/.forgejo-token",
    })
}

fn command(material: &shaula_core::ports::forgejo::ForgejoBootstrapMaterial) -> Value {
    let mut command = vec!["/bin/forgejo-runner".to_string()];
    command.extend(material.one_job_args());
    json!(command)
}

#[cfg(test)]
#[path = "runtime_bootstrap_docker_forgejo_tests.rs"]
mod tests;
