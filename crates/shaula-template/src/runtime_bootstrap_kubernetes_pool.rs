//! Shared Pending-Pod proof for Kubernetes Pool backends (Forgejo, GitLab):
//! the official image runs a fixed one-job command as a pinned non-root user,
//! gated by one required Secret item. The backend supplies the launch shape;
//! the saved-plan admission reuses the same shape.
use super::{binding, failed, protected_file, resource, Commands};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use shaula_core::{
    ports::{TemplateCreateRequest, TemplateOutcomeError},
    template::ShaulaResultEnvelope,
};
use std::{path::Path, time::Duration};

/// The exact Pod a backend's Terraform branch must have produced.
pub(in crate::runtime::bootstrap) struct PoolPod<'a> {
    /// Nonsecret `runner_backend` marker in the Secret before release.
    pub(in crate::runtime::bootstrap) backend: &'static str,
    pub(in crate::runtime::bootstrap) command: [&'static str; 3],
    pub(in crate::runtime::bootstrap) args: Vec<String>,
    pub(in crate::runtime::bootstrap) run_as_user: u64,
    /// `None` leaves the image default (retained Forgejo revisions).
    pub(in crate::runtime::bootstrap) working_dir: Option<&'static str>,
    /// Secret key, item path and subPath of the startup gate.
    pub(in crate::runtime::bootstrap) key: &'static str,
    pub(in crate::runtime::bootstrap) mount_path: &'static str,
    /// Writable Generation-owned emptyDir mount, when the runner needs one.
    pub(in crate::runtime::bootstrap) scratch: Option<&'static str>,
    /// Credential bytes that must never appear before the release patch.
    pub(in crate::runtime::bootstrap) secret: &'a str,
    /// Value patched under `key`.
    pub(in crate::runtime::bootstrap) payload: String,
}

pub(super) async fn launch(
    request: &TemplateCreateRequest,
    envelope: &ShaulaResultEnvelope,
    image: &str,
    temporary: &Path,
    timeout: Duration,
    shape: &PoolPod<'_>,
) -> Result<(), TemplateOutcomeError> {
    let namespace = binding(request, "namespace")?;
    let name = &request.input.generation.generation_name;
    let runner = resource(envelope, "runner")?;
    let bootstrap = resource(envelope, "bootstrap")?;
    let expected_id = format!("{namespace}/{name}");
    if runner.id != expected_id || bootstrap.id != expected_id {
        return Err(failed());
    }
    let commands = Commands::new(
        "kubectl",
        &request.workspace_path,
        super::kubectl_prefix(request, temporary)?,
        timeout,
    )?;
    let pod = commands.json(&["get", "pod", name, "-o", "json"]).await?;
    super::validate_metadata(&pod, runner, request, "Pod")?;
    validate_pod(&pod, image, request, shape)?;
    let secret = commands
        .json(&["get", "secret", name, "-o", "json"])
        .await?;
    super::validate_metadata(&secret, bootstrap, request, "Secret")?;
    let patch = patch(&secret, shape)?;
    let file = protected_file(
        temporary,
        &serde_json::to_vec(&patch).map_err(|_| failed())?,
        false,
    )?;
    let path = file.path().to_str().ok_or_else(failed)?;
    commands
        .run(&["patch", "secret", name, "--type=json", "--patch-file", path])
        .await?;
    Ok(())
}

pub(super) fn validate_pod(
    pod: &Value,
    image: &str,
    request: &TemplateCreateRequest,
    shape: &PoolPod<'_>,
) -> Result<(), TemplateOutcomeError> {
    let spec = &pod["spec"];
    let containers = spec["containers"]
        .as_array()
        .filter(|values| values.len() == 1)
        .ok_or_else(failed)?;
    let runner = containers.first().ok_or_else(failed)?;
    let mounts = runner["volumeMounts"].as_array().ok_or_else(failed)?;
    let volumes = spec["volumes"].as_array().ok_or_else(failed)?;
    let expected = 1 + usize::from(shape.scratch.is_some());
    let mount = mounts
        .iter()
        .find(|mount| mount["mountPath"] == shape.mount_path)
        .ok_or_else(failed)?;
    let volume = volumes
        .iter()
        .find(|volume| volume["name"] == mount["name"])
        .ok_or_else(failed)?;
    let previously_started =
        pod["status"]["containerStatuses"]
            .as_array()
            .is_some_and(|statuses| {
                statuses.iter().any(|status| {
                    status["started"] == true
                        || status["restartCount"]
                            .as_u64()
                            .is_some_and(|count| count > 0)
                        || status["containerID"]
                            .as_str()
                            .is_some_and(|id| !id.is_empty())
                        || !status["state"]["running"].is_null()
                        || !status["state"]["terminated"].is_null()
                        || !status["lastState"]["terminated"].is_null()
                })
            });
    if shape.secret.is_empty()
        || pod.to_string().contains(shape.secret)
        || pod["status"]["phase"] != "Pending"
        || previously_started
        || spec["restartPolicy"] != "Never"
        || spec["automountServiceAccountToken"] != false
        || ["hostNetwork", "hostPID", "hostIPC"]
            .iter()
            .any(|key| spec[*key] == true)
        || !spec["ephemeralContainers"].is_null() && spec["ephemeralContainers"] != json!([])
        || runner["securityContext"]["runAsNonRoot"] != true
        || runner["securityContext"]["runAsUser"] != shape.run_as_user
        || runner["securityContext"]["allowPrivilegeEscalation"] != false
        || runner["securityContext"]["privileged"] == true
        || runner["securityContext"]["capabilities"]["drop"] != json!(["ALL"])
        || !runner["securityContext"]["capabilities"]["add"].is_null()
            && runner["securityContext"]["capabilities"]["add"] != json!([])
        || !spec["initContainers"].is_null() && spec["initContainers"] != json!([])
        || runner["name"] != "runner"
        || runner["image"] != image
        || runner["command"] != json!(shape.command)
        || shape
            .working_dir
            .is_some_and(|directory| runner["workingDir"] != directory)
        || mounts.len() != expected
        || volumes.len() != expected
        || runner["args"] != json!(shape.args)
        || !runner["env"].is_null() && runner["env"] != json!([])
        || runner["envFrom"] != json!([]) && !runner["envFrom"].is_null()
        || mount["subPath"] != shape.key
        || mount["readOnly"] != true
        || volume["secret"]["secretName"] != request.input.generation.generation_name
        || volume["secret"]["optional"] == true
        || volume["secret"]["items"].as_array().is_none_or(|items| {
            items.len() != 1 || items[0]["key"] != shape.key || items[0]["path"] != shape.key
        })
        || shape
            .scratch
            .is_some_and(|path| !scratch(mounts, volumes, path))
    {
        return Err(failed());
    }
    Ok(())
}

// The only other mount is a plain, writable emptyDir; never a host path,
// another Secret or a projected service-account token.
fn scratch(mounts: &[Value], volumes: &[Value], path: &str) -> bool {
    let Some(mount) = mounts.iter().find(|mount| mount["mountPath"] == path) else {
        return false;
    };
    let Some(volume) = volumes
        .iter()
        .find(|volume| volume["name"] == mount["name"])
    else {
        return false;
    };
    let medium = &volume["emptyDir"]["medium"];
    (mount["subPath"].is_null() || mount["subPath"] == "")
        && mount["readOnly"] != true
        && (mount["mountPropagation"].is_null() || mount["mountPropagation"] == "None")
        && volume["emptyDir"].is_object()
        && (medium.is_null() || medium == "")
        && volume
            .as_object()
            .is_some_and(|fields| fields.keys().all(|key| key == "name" || key == "emptyDir"))
}

pub(super) fn patch(secret: &Value, shape: &PoolPod<'_>) -> Result<Value, TemplateOutcomeError> {
    let marker = json!({"runner_backend":STANDARD.encode(shape.backend)});
    if shape.secret.is_empty()
        || (!secret["data"].is_null() && secret["data"] != json!({}) && secret["data"] != marker)
        || secret.to_string().contains(shape.secret)
        || (!secret["immutable"].is_null() && secret["immutable"] != false)
        || secret["type"] != "Opaque"
    {
        return Err(failed());
    }
    Ok(json!([
        {"op":"test", "path":"/metadata/uid", "value":secret["metadata"]["uid"]},
        {"op":"test", "path":"/metadata/resourceVersion", "value":secret["metadata"]["resourceVersion"]},
        {"op":"add", "path":"/data", "value":{shape.key:STANDARD.encode(&shape.payload)}},
        {"op":"add", "path":"/immutable", "value":true}
    ]))
}
