use super::{binding, failed, protected_file, resource, Commands};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use shaula_core::{
    ports::{TemplateCreateRequest, TemplateOutcomeError},
    template::ShaulaResultEnvelope,
};
use std::{path::Path, time::Duration};

pub(super) async fn launch(
    request: &TemplateCreateRequest,
    envelope: &ShaulaResultEnvelope,
    image: &str,
    temporary: &Path,
    timeout: Duration,
) -> Result<(), TemplateOutcomeError> {
    let namespace = binding(request, "namespace")?;
    let kubeconfig = binding(request, "kubeconfig")?;
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
        vec![
            "--kubeconfig".into(),
            kubeconfig.into(),
            "--namespace".into(),
            namespace.into(),
            "--request-timeout=15s".into(),
        ],
        timeout,
    )?;
    let pod = commands.json(&["get", "pod", name, "-o", "json"]).await?;
    super::validate_metadata(&pod, runner, request, "Pod")?;
    validate_pod(&pod, image, request)?;
    let secret = commands
        .json(&["get", "secret", name, "-o", "json"])
        .await?;
    super::validate_metadata(&secret, bootstrap, request, "Secret")?;
    let patch = patch(&secret, request)?;
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

fn validate_pod(
    pod: &Value,
    image: &str,
    request: &TemplateCreateRequest,
) -> Result<(), TemplateOutcomeError> {
    let material = request.forgejo_bootstrap.as_ref().ok_or_else(failed)?;
    let spec = &pod["spec"];
    let containers = spec["containers"]
        .as_array()
        .filter(|values| values.len() == 1)
        .ok_or_else(failed)?;
    let runner = containers.first().ok_or_else(failed)?;
    let mounts = runner["volumeMounts"].as_array().ok_or_else(failed)?;
    let mount = mounts
        .iter()
        .find(|mount| mount["mountPath"] == "/data/.forgejo-token")
        .ok_or_else(failed)?;
    let volumes = spec["volumes"].as_array().ok_or_else(failed)?;
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
    if pod["status"]["phase"] != "Pending"
        || previously_started
        || spec["restartPolicy"] != "Never"
        || spec["automountServiceAccountToken"] != false
        || ["hostNetwork", "hostPID", "hostIPC"]
            .iter()
            .any(|key| spec[*key] == true)
        || !spec["ephemeralContainers"].is_null() && spec["ephemeralContainers"] != json!([])
        || runner["securityContext"]["runAsNonRoot"] != true
        || runner["securityContext"]["runAsUser"] != 1000
        || runner["securityContext"]["allowPrivilegeEscalation"] != false
        || runner["securityContext"]["privileged"] == true
        || runner["securityContext"]["capabilities"]["drop"] != json!(["ALL"])
        || !runner["securityContext"]["capabilities"]["add"].is_null()
            && runner["securityContext"]["capabilities"]["add"] != json!([])
        || !spec["initContainers"].is_null() && spec["initContainers"] != json!([])
        || runner["name"] != "runner"
        || runner["image"] != image
        || runner["command"] != json!(["/usr/bin/dumb-init", "--", "/bin/forgejo-runner"])
        || mounts.len() != 1
        || volumes.len() != 1
        || runner["args"] != command(material)
        || !runner["env"].is_null() && runner["env"] != json!([])
        || runner["envFrom"] != json!([]) && !runner["envFrom"].is_null()
        || mount["subPath"] != "token"
        || mount["readOnly"] != true
        || volume["secret"]["secretName"] != request.input.generation.generation_name
        || volume["secret"]["optional"] == true
        || volume["secret"]["items"].as_array().is_none_or(|items| {
            items.len() != 1 || items[0]["key"] != "token" || items[0]["path"] != "token"
        })
    {
        return Err(failed());
    }
    Ok(())
}

fn patch(secret: &Value, request: &TemplateCreateRequest) -> Result<Value, TemplateOutcomeError> {
    let material = request.forgejo_bootstrap.as_ref().ok_or_else(failed)?;
    if (!secret["data"].is_null()
        && secret["data"]
            .as_object()
            .is_none_or(|data| !data.is_empty()))
        || secret.to_string().contains(material.token())
        || (!secret["immutable"].is_null() && secret["immutable"] != false)
        || secret["type"] != "Opaque"
    {
        return Err(failed());
    }
    Ok(json!([
        {"op":"test", "path":"/metadata/uid", "value":secret["metadata"]["uid"]},
        {"op":"test", "path":"/metadata/resourceVersion", "value":secret["metadata"]["resourceVersion"]},
        {"op":"add", "path":"/data", "value":{"token":STANDARD.encode(material.token())}},
        {"op":"add", "path":"/immutable", "value":true}
    ]))
}

#[cfg(test)]
#[path = "runtime_bootstrap_kubernetes_forgejo_tests.rs"]
mod tests;

fn command(material: &shaula_core::ports::forgejo::ForgejoBootstrapMaterial) -> Value {
    json!(material.one_job_args())
}
