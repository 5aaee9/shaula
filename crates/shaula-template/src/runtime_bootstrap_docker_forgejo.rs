use super::{failed, protected_file, TemplateCreateRequest, TemplateOutcomeError};
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
    let material = request.forgejo_bootstrap.as_ref().ok_or_else(failed)?;
    let directory = temporary.ok_or_else(failed)?;
    let token = protected_file(directory, material.token().as_bytes(), true)?;
    let path = token.path().to_str().ok_or_else(failed)?;
    let destination = format!("{id}:/data/.forgejo-token");
    commands.run(&["cp", "--", path, &destination]).await?;

    let info = commands.json(&["container", "inspect", "--", id]).await?;
    validate(single(&info)?, id, image_id, request)?;
    commands.run(&["container", "start", "--", id]).await?;
    Ok(())
}

pub(super) fn validate(
    info: &Value,
    id: &str,
    image_id: &str,
    request: &TemplateCreateRequest,
) -> Result<(), TemplateOutcomeError> {
    let material = request.forgejo_bootstrap.as_ref().ok_or_else(failed)?;
    if material.instance_url.contains('\0')
        || material.uuid.contains('\0')
        || material.token().contains('\0')
        || info.to_string().contains(material.token())
    {
        return Err(failed());
    }
    let identity = &request.input.generation;
    let short: String = identity.id.chars().take(24).collect();
    let name = format!("/shaula-{}-{short}", identity.fleet_key);
    let config = &info["Config"];
    let state = &info["State"];
    let host = &info["HostConfig"];
    let environment = config["Env"].as_array().ok_or_else(failed)?;
    let command = command(material);
    let user = config["User"]
        .as_str()
        .unwrap_or_default()
        .split(':')
        .next()
        .unwrap_or_default();
    if info["Id"] != id
        || info["Name"] != name
        || info["Image"] != image_id
        || config["Cmd"] != command
        || config["Entrypoint"] != json!(["/usr/bin/dumb-init"])
        || config["WorkingDir"] != "/data"
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
        || !official_data_volume(info)
        || [
            "Binds",
            "Mounts",
            "VolumesFrom",
            "CapAdd",
            "Devices",
            "DeviceRequests",
        ]
        .iter()
        .any(|key| !host[*key].is_null() && host[*key] != json!([]))
        || ["PidMode", "IpcMode", "NetworkMode", "UTSMode", "UsernsMode"]
            .iter()
            .any(|key| {
                host[*key]
                    .as_str()
                    .is_some_and(|value| value == "host" || value.starts_with("container:"))
            })
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

// The pinned upstream image declares VOLUME /data. Docker creates one
// anonymous execution-domain volume even when the plan requests no mounts.
// Binds, named mounts and volumes-from remain forbidden above.
fn official_data_volume(info: &Value) -> bool {
    let Some(mounts) = info["Mounts"].as_array() else {
        return false;
    };
    let [mount] = mounts.as_slice() else {
        return false;
    };
    info["Config"]["Volumes"] == json!({"/data": {}})
        && mount["Type"] == "volume"
        && mount["Destination"] == "/data"
        && mount["Driver"] == "local"
        && mount["RW"] == true
        && mount["Name"].as_str().is_some_and(|name| {
            name.len() == 64 && name.bytes().all(|byte| byte.is_ascii_hexdigit())
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

fn single(value: &Value) -> Result<&Value, TemplateOutcomeError> {
    let values = value
        .as_array()
        .filter(|values| values.len() == 1)
        .ok_or_else(failed)?;
    values.first().ok_or_else(failed)
}
