use super::{binding, failed, protected_file, resource, Commands, LISTENER};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use shaula_core::{
    ports::{TemplateCreateRequest, TemplateOutcomeError},
    template::{ResultResource, ShaulaResultEnvelope},
};
use std::{path::Path, time::Duration};

pub(super) async fn launch(
    request: &TemplateCreateRequest,
    envelope: &ShaulaResultEnvelope,
    image: &str,
    setup: &[u8],
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
    validate_metadata(&pod, runner, request, "Pod")?;
    validate_pod(&pod, image, request)?;
    let secret = commands
        .json(&["get", "secret", name, "-o", "json"])
        .await?;
    validate_metadata(&secret, bootstrap, request, "Secret")?;
    let patch = patch(&secret, setup, request)?;
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

pub(super) fn validate_metadata(
    value: &Value,
    resource: &ResultResource,
    request: &TemplateCreateRequest,
    kind: &str,
) -> Result<(), TemplateOutcomeError> {
    let identity = &request.input.generation;
    let metadata = &value["metadata"];
    let uid = resource
        .incarnation
        .as_deref()
        .filter(|uid| uuid::Uuid::parse_str(uid).is_ok())
        .ok_or_else(failed)?;
    let version = metadata["resourceVersion"].as_str().ok_or_else(failed)?;
    if value["kind"] != kind
        || value["apiVersion"] != "v1"
        || metadata["uid"] != uid
        || metadata["name"] != identity.generation_name
        || metadata["namespace"] != binding(request, "namespace")?
        || metadata["labels"]["shaula.io/fleet"] != identity.fleet_key
        || metadata["labels"]["shaula.io/generation-id"] != identity.id
        || !metadata["deletionTimestamp"].is_null()
        || version.is_empty()
        || version.len() > 512
        || version.contains('\0')
    {
        return Err(failed());
    }
    Ok(())
}

pub(super) fn validate_pod(
    pod: &Value,
    image: &str,
    request: &TemplateCreateRequest,
) -> Result<(), TemplateOutcomeError> {
    let spec = &pod["spec"];
    let containers = spec["containers"]
        .as_array()
        .filter(|values| values.len() == 1)
        .ok_or_else(failed)?;
    let runner = containers.first().ok_or_else(failed)?;
    let environment = runner["env"].as_array().ok_or_else(failed)?;
    let jit_entries: Vec<_> = environment
        .iter()
        .filter(|entry| {
            entry["name"].as_str().is_some_and(|name| {
                name.to_ascii_uppercase()
                    .starts_with("ACTIONS_RUNNER_INPUT_")
            })
        })
        .collect();
    let jit = jit_entries.first().ok_or_else(failed)?;
    let secret = &jit["valueFrom"]["secretKeyRef"];
    let mounts = runner["volumeMounts"].as_array().ok_or_else(failed)?;
    let mount = mounts
        .iter()
        .find(|mount| mount["mountPath"] == "/home/runner/.setup_info")
        .ok_or_else(failed)?;
    let volumes = spec["volumes"].as_array().ok_or_else(failed)?;
    let volume = volumes
        .iter()
        .find(|volume| volume["name"] == mount["name"])
        .ok_or_else(failed)?;
    let source = &volume["secret"];
    let name = &request.input.generation.generation_name;
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
        || !spec["initContainers"].is_null() && spec["initContainers"] != json!([])
        || runner["name"] != "runner"
        || runner["image"] != image
        || runner["command"] != json!([LISTENER, "run"])
        || !runner["args"].is_null() && runner["args"] != json!([])
        || jit_entries.len() != 1
        || jit["name"] != "ACTIONS_RUNNER_INPUT_JITCONFIG"
        || !jit["value"].is_null()
        || secret["name"] != *name
        || secret["key"] != "jit_config"
        || secret["optional"] == true
        || mount["subPath"] != ".setup_info"
        || mount["readOnly"] != true
        || source["secretName"] != *name
        || source["optional"] == true
        || source["items"].as_array().is_none_or(|items| {
            items.len() != 1
                || items[0]["key"] != ".setup_info"
                || items[0]["path"] != ".setup_info"
        })
    {
        return Err(failed());
    }
    Ok(())
}

pub(super) fn patch(
    secret: &Value,
    setup: &[u8],
    request: &TemplateCreateRequest,
) -> Result<Value, TemplateOutcomeError> {
    let data = secret["data"]
        .as_object()
        .filter(|data| data.len() == 1)
        .ok_or_else(failed)?;
    let expected_jit = STANDARD.encode(&request.input.jit_config);
    if (!secret["immutable"].is_null() && secret["immutable"] != false)
        || secret["type"] != "Opaque"
        || data.get("jit_config").and_then(Value::as_str) != Some(&expected_jit)
    {
        return Err(failed());
    }
    // The API limits the sum of decoded Secret values, including the actual
    // frozen JIT payload. Diagnostics cannot turn an otherwise valid JIT into
    // an oversized Secret, and bootstrap never truncates or replaces JIT.
    let available = (1024 * 1024_usize)
        .checked_sub(request.input.jit_config.len())
        .ok_or_else(failed)?;
    let setup = if setup.len() <= available {
        setup
    } else if available >= 2 {
        b"[]"
    } else {
        return Err(failed());
    };
    Ok(json!([
        {"op":"test", "path":"/metadata/uid", "value":secret["metadata"]["uid"]},
        {"op":"test", "path":"/metadata/resourceVersion", "value":secret["metadata"]["resourceVersion"]},
        {"op":"add", "path":"/data/.setup_info", "value":STANDARD.encode(setup)},
        {"op":"add", "path":"/immutable", "value":true}
    ]))
}
