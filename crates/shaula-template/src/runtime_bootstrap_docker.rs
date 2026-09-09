use super::{binding, failed, protected_file, resource, Commands, LISTENER};
use serde_json::Value;
use shaula_core::{
    ports::{TemplateCreateRequest, TemplateOutcomeError},
    template::ShaulaResultEnvelope,
};
use std::{path::Path, time::Duration};

pub(super) async fn launch(
    request: &TemplateCreateRequest,
    envelope: &ShaulaResultEnvelope,
    image: &str,
    setup: &[u8],
    temporary: Option<&Path>,
    timeout: Duration,
) -> Result<(), TemplateOutcomeError> {
    let host = request
        .input
        .bindings
        .get("docker_host")
        .map(|_| binding(request, "docker_host"))
        .transpose()?
        .unwrap_or("unix:///var/run/docker.sock");
    if !local_host(host) {
        return Err(failed());
    }
    let runner = resource(envelope, "runner")?;
    if runner.id.len() != 64 || !runner.id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(failed());
    }
    let commands = Commands::new(
        "docker",
        &request.workspace_path,
        vec!["--host".into(), host.into()],
        timeout,
    )?;
    let image_info = commands.json(&["image", "inspect", "--", image]).await?;
    let image_id = single(&image_info)?
        .get("Id")
        .and_then(Value::as_str)
        .ok_or_else(failed)?;
    let info = commands
        .json(&["container", "inspect", "--", &runner.id])
        .await?;
    validate(single(&info)?, &runner.id, image_id, request)?;

    // Diagnostics have their own failure boundary. JIT was frozen in the
    // stopped container by Terraform; an optional copy cannot bypass start.
    if let Some(file) = temporary.and_then(|directory| protected_file(directory, setup, true).ok())
    {
        if let Some(path) = file.path().to_str() {
            let destination = format!("{}:/home/runner/.setup_info", runner.id);
            commands.optional(&["cp", "--", path, &destination]).await;
        }
    }
    let info = commands
        .json(&["container", "inspect", "--", &runner.id])
        .await?;
    validate(single(&info)?, &runner.id, image_id, request)?;
    commands
        .run(&["container", "start", "--", &runner.id])
        .await?;
    Ok(())
}

pub(super) fn local_host(host: &str) -> bool {
    host.starts_with("unix:///")
        || host.strip_prefix("npipe:////./pipe/").is_some_and(|name| {
            !name.is_empty() && !name.contains(['/', '\\', ':']) && name != "." && name != ".."
        })
}

fn single(value: &Value) -> Result<&Value, TemplateOutcomeError> {
    let values = value
        .as_array()
        .filter(|values| values.len() == 1)
        .ok_or_else(failed)?;
    values.first().ok_or_else(failed)
}

pub(super) fn validate(
    info: &Value,
    id: &str,
    image_id: &str,
    request: &TemplateCreateRequest,
) -> Result<(), TemplateOutcomeError> {
    let identity = &request.input.generation;
    let short: String = identity.id.chars().take(24).collect();
    let name = format!("/shaula-{}-{short}", identity.fleet_key);
    let config = &info["Config"];
    let state = &info["State"];
    let host = &info["HostConfig"];
    let environment = config["Env"].as_array().ok_or_else(failed)?;
    let jit = format!(
        "ACTIONS_RUNNER_INPUT_JITCONFIG={}",
        request.input.jit_config
    );
    let user = config["User"]
        .as_str()
        .unwrap_or_default()
        .split(':')
        .next()
        .unwrap_or_default();
    if info["Id"] != id
        || info["Name"] != name
        || info["Image"] != image_id
        || config["Cmd"] != serde_json::json!([LISTENER, "run"])
        || !config["Entrypoint"].is_null() && config["Entrypoint"] != serde_json::json!([])
        || config["Labels"]["shaula.fleet"] != identity.fleet_key
        || config["Labels"]["shaula.generation"] != identity.id
        || state["Status"] != "created"
        || state["Running"] != false
        || state["Restarting"] != false
        || state["Dead"] != false
        || host["RestartPolicy"]["Name"] != "no"
        || host["AutoRemove"] != false
        || host["Privileged"] != false
        || user.is_empty()
        || user == "root"
        || user.bytes().all(|byte| byte == b'0')
        || !info["Mounts"].is_null() && info["Mounts"] != serde_json::json!([])
        || [
            "Binds",
            "VolumesFrom",
            "CapAdd",
            "Devices",
            "DeviceRequests",
        ]
        .iter()
        .any(|key| !host[*key].is_null() && host[*key] != serde_json::json!([]))
        || ["PidMode", "IpcMode", "NetworkMode", "UTSMode", "UsernsMode"]
            .iter()
            .any(|key| {
                host[*key]
                    .as_str()
                    .is_some_and(|value| value == "host" || value.starts_with("container:"))
            })
        || environment
            .iter()
            .filter(|entry| entry.as_str() == Some(&jit))
            .count()
            != 1
        || environment.iter().filter_map(Value::as_str).any(|entry| {
            entry
                .to_ascii_uppercase()
                .starts_with("ACTIONS_RUNNER_INPUT_")
                && entry != jit
        })
    {
        return Err(failed());
    }
    Ok(())
}
