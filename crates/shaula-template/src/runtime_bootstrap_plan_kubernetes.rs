use super::{rejected, Planned, TemplateCreateRequest, TemplateOutcomeError};

#[path = "runtime_bootstrap_plan_kubernetes_forgejo.rs"]
mod forgejo;
use serde_json::json;

pub(super) fn admit(
    pod: &Planned<'_>,
    secret: &Planned<'_>,
    image: &str,
    request: &TemplateCreateRequest,
    backend: &str,
) -> Result<(), TemplateOutcomeError> {
    if backend == "forgejo" {
        return forgejo::admit(pod, secret, image, request);
    }
    let identity = &request.input.generation;
    let namespace = super::super::binding(request, "namespace").map_err(|_| rejected())?;
    for resource in [pod, secret] {
        resource.one("/metadata")?;
        resource.exact("/metadata/0/name", &json!(identity.generation_name))?;
        resource.exact("/metadata/0/namespace", &json!(namespace))?;
        resource.exact(
            "/metadata/0/labels",
            &json!({
                "shaula.io/fleet":identity.fleet_key,
                "shaula.io/generation-id":identity.id
            }),
        )?;
        resource.empty("/metadata/0/generate_name")?;
    }
    secret.exact("/immutable", &json!(false))?;
    secret.exact("/type", &json!("Opaque"))?;
    secret.exact("/data", &json!({"jit_config":request.input.jit_config}))?;
    // Both attributes become API Secret data. A binary_data key would release
    // the same required-key gate just as surely as an ordinary data entry.
    secret.empty("/binary_data")?;
    pod.exact("/target_state", &json!(["Pending"]))?;
    pod.one("/spec")?;
    pod.exact("/spec/0/restart_policy", &json!("Never"))?;
    pod.exact("/spec/0/automount_service_account_token", &json!(false))?;
    pod.empty("/spec/0/init_container")?;
    pod.one("/spec/0/container")?;
    let runner = "/spec/0/container/0";
    for (field, value) in [
        ("name", json!("runner")),
        ("image", json!(image)),
        ("command", json!([super::super::LISTENER, "run"])),
    ] {
        pod.exact(&format!("{runner}/{field}"), &value)?;
    }
    for field in ["args", "env_from", "lifecycle", "volume_device"] {
        pod.empty(&format!("{runner}/{field}"))?;
    }
    pod.one(&format!("{runner}/env"))?;
    let env = format!("{runner}/env/0");
    pod.exact(
        &format!("{env}/name"),
        &json!("ACTIONS_RUNNER_INPUT_JITCONFIG"),
    )?;
    pod.empty(&format!("{env}/value"))?;
    pod.one(&format!("{env}/value_from"))?;
    let from = format!("{env}/value_from/0");
    for source in ["config_map_key_ref", "field_ref", "resource_field_ref"] {
        pod.empty(&format!("{from}/{source}"))?;
    }
    pod.one(&format!("{from}/secret_key_ref"))?;
    let reference = format!("{from}/secret_key_ref/0");
    for (field, value) in [
        ("name", json!(identity.generation_name)),
        ("key", json!("jit_config")),
        ("optional", json!(false)),
    ] {
        pod.exact(&format!("{reference}/{field}"), &value)?;
    }
    pod.one(&format!("{runner}/volume_mount"))?;
    let mount = format!("{runner}/volume_mount/0");
    for (field, value) in [
        ("mount_path", json!("/home/runner/.setup_info")),
        ("sub_path", json!(".setup_info")),
        ("read_only", json!(true)),
    ] {
        pod.exact(&format!("{mount}/{field}"), &value)?;
    }
    let volume_name = pod
        .known(&format!("{mount}/name"))?
        .as_str()
        .filter(|name| !name.is_empty())
        .ok_or_else(rejected)?;
    pod.one("/spec/0/volume")?;
    pod.exact("/spec/0/volume/0/name", &json!(volume_name))?;
    pod.one("/spec/0/volume/0/secret")?;
    let source = "/spec/0/volume/0/secret/0";
    pod.exact(
        &format!("{source}/secret_name"),
        &json!(identity.generation_name),
    )?;
    pod.exact(&format!("{source}/optional"), &json!(false))?;
    pod.one(&format!("{source}/items"))?;
    pod.exact(&format!("{source}/items/0/key"), &json!(".setup_info"))?;
    pod.exact(&format!("{source}/items/0/path"), &json!(".setup_info"))?;
    // A single Secret source is required; disallow an alternative volume kind
    // that happens to contain a matching field in a malformed/unknown plan.
    let volume = pod
        .value("/spec/0/volume/0")?
        .as_object()
        .ok_or_else(rejected)?;
    for key in volume
        .keys()
        .filter(|key| !matches!(key.as_str(), "name" | "secret"))
    {
        pod.empty(&format!("/spec/0/volume/0/{key}"))?;
    }
    Ok(())
}
