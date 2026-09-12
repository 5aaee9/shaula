use super::{rejected, Planned, TemplateCreateRequest, TemplateOutcomeError};
use serde_json::{json, Value};

pub(super) fn admit(
    pod: &Planned<'_>,
    secret: &Planned<'_>,
    image: &str,
    request: &TemplateCreateRequest,
) -> Result<(), TemplateOutcomeError> {
    let material = request.forgejo_bootstrap.as_ref().ok_or_else(rejected)?;
    let identity = &request.input.generation;
    let namespace = super::super::super::binding(request, "namespace").map_err(|_| rejected())?;
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
    }
    secret.exact("/immutable", &json!(false))?;
    secret.exact("/type", &json!("Opaque"))?;
    // An empty Secret is an intentional start gate: the pod cannot obtain its
    // required token until the host-owned post-apply patch succeeds.
    secret.exact("/data", &json!({}))?;
    secret.empty("/binary_data")?;

    pod.exact("/target_state", &json!(["Pending"]))?;
    pod.one("/spec")?;
    pod.exact("/spec/0/restart_policy", &json!("Never"))?;
    pod.exact("/spec/0/automount_service_account_token", &json!(false))?;
    pod.empty("/spec/0/init_container")?;
    pod.one("/spec/0/container")?;
    for field in ["host_network", "host_pid", "host_ipc"] {
        let value = pod.known(&format!("/spec/0/{field}"))?;
        if !value.is_null() && value != false {
            return Err(rejected());
        }
    }
    let runner = "/spec/0/container/0";
    pod.one(&format!("{runner}/security_context"))?;
    let security = format!("{runner}/security_context/0");
    pod.exact(&format!("{security}/run_as_non_root"), &json!(true))?;
    pod.exact(&format!("{security}/run_as_user"), &json!(1000))?;
    pod.exact(
        &format!("{security}/allow_privilege_escalation"),
        &json!(false),
    )?;
    let privileged = pod.known(&format!("{security}/privileged"))?;
    if !privileged.is_null() && privileged != false {
        return Err(rejected());
    }
    pod.one(&format!("{security}/capabilities"))?;
    pod.exact(&format!("{security}/capabilities/0/drop"), &json!(["ALL"]))?;
    pod.empty(&format!("{security}/capabilities/0/add"))?;
    pod.exact(&format!("{runner}/name"), &json!("runner"))?;
    pod.exact(&format!("{runner}/image"), &json!(image))?;
    pod.exact(
        &format!("{runner}/command"),
        &json!(["/usr/bin/dumb-init", "--", "/bin/forgejo-runner"]),
    )?;
    pod.exact(&format!("{runner}/args"), &command(material))?;
    for field in ["env", "env_from", "lifecycle", "volume_device"] {
        pod.empty(&format!("{runner}/{field}"))?;
    }
    pod.one(&format!("{runner}/volume_mount"))?;
    let mount = format!("{runner}/volume_mount/0");
    pod.exact(
        &format!("{mount}/mount_path"),
        &json!("/data/.forgejo-token"),
    )?;
    pod.exact(&format!("{mount}/sub_path"), &json!("token"))?;
    pod.exact(&format!("{mount}/read_only"), &json!(true))?;
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
    pod.exact(&format!("{source}/items/0/key"), &json!("token"))?;
    pod.exact(&format!("{source}/items/0/path"), &json!("token"))?;
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

fn command(material: &shaula_core::ports::forgejo::ForgejoBootstrapMaterial) -> Value {
    json!(material.one_job_args())
}
