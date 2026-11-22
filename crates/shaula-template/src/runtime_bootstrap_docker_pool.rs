//! Shared stopped-container proof for Pool backends (Forgejo, GitLab): the
//! official image runs a fixed one-job command, as a non-root user, with only
//! the image's own anonymous volumes. The backend supplies the launch shape.
use super::{failed, protected_file, TemplateCreateRequest, TemplateOutcomeError};
use serde_json::{json, Value};
use std::path::Path;

use super::Commands;

/// The exact launch a backend's Terraform branch must have produced.
pub(super) struct PoolLaunch<'a> {
    pub command: Value,
    pub working_dir: &'a str,
    /// `None` accepts any non-root image user; `Some` requires that name.
    pub user: Option<&'a str>,
    /// Destinations of the image-declared `VOLUME`s, in any order.
    pub volumes: &'a [&'a str],
    /// Credential bytes that must never appear in container metadata.
    pub secret: &'a str,
    /// Where the protected payload is copied before start.
    pub destination: &'a str,
}

/// Copies the protected credential `payload`, re-proves the container and starts it.
pub(super) async fn launch(
    commands: &Commands<'_>,
    id: &str,
    image_id: &str,
    request: &TemplateCreateRequest,
    temporary: Option<&Path>,
    shape: &PoolLaunch<'_>,
    payload: &str,
) -> Result<(), TemplateOutcomeError> {
    let directory = temporary.ok_or_else(failed)?;
    let file = protected_file(directory, payload.as_bytes(), true)?;
    let path = file.path().to_str().ok_or_else(failed)?;
    let destination = format!("{id}:{}", shape.destination);
    commands.run(&["cp", "--", path, &destination]).await?;

    let info = commands.json(&["container", "inspect", "--", id]).await?;
    validate(super::single(&info)?, id, image_id, request, shape)?;
    commands.run(&["container", "start", "--", id]).await?;
    Ok(())
}

pub(super) fn validate(
    info: &Value,
    id: &str,
    image_id: &str,
    request: &TemplateCreateRequest,
    shape: &PoolLaunch<'_>,
) -> Result<(), TemplateOutcomeError> {
    if shape.secret.is_empty() || info.to_string().contains(shape.secret) {
        return Err(failed());
    }
    let (config, environment, user) = super::validate_common(info, id, image_id, request)?;
    let host = &info["HostConfig"];
    if config["Cmd"] != shape.command
        || config["Entrypoint"] != json!(["/usr/bin/dumb-init"])
        || config["WorkingDir"] != shape.working_dir
        || shape.user.is_some_and(|expected| user != expected)
        || !image_volumes(info, shape.volumes)
        || (!host["Mounts"].is_null() && host["Mounts"] != json!([]))
        || environment.iter().filter_map(Value::as_str).any(|entry| {
            entry
                .to_ascii_uppercase()
                .starts_with("ACTIONS_RUNNER_INPUT_")
        })
    {
        return Err(failed());
    }
    Ok(())
}

// The pinned upstream images declare VOLUMEs. Docker creates one anonymous
// execution-domain volume per declaration even when the plan requests no
// mounts. Binds, named mounts and volumes-from remain forbidden above.
fn image_volumes(info: &Value, destinations: &[&str]) -> bool {
    let Some(mounts) = info["Mounts"].as_array() else {
        return false;
    };
    let declared: serde_json::Map<String, Value> = destinations
        .iter()
        .map(|destination| ((*destination).to_owned(), json!({})))
        .collect();
    info["Config"]["Volumes"] == Value::Object(declared)
        && mounts.len() == destinations.len()
        && destinations.iter().all(|destination| {
            mounts
                .iter()
                .filter(|mount| mount["Destination"] == *destination)
                .count()
                == 1
        })
        && mounts.iter().all(|mount| {
            mount["Type"] == "volume"
                && mount["Driver"] == "local"
                && mount["RW"] == true
                && mount["Name"].as_str().is_some_and(|name| {
                    name.len() == 64 && name.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
        })
}
