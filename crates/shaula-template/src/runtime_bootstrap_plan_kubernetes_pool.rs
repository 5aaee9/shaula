//! Saved-plan admission shared by the Kubernetes Pool backends (Forgejo,
//! GitLab). The backend's launch shape fixes command, user and mounts.
use super::{rejected, Planned, TemplateCreateRequest, TemplateOutcomeError};
use serde_json::json;

use super::super::super::kubernetes::pool_shape;

pub(super) fn admit(
    pod: &Planned<'_>,
    secret: &Planned<'_>,
    image: &str,
    request: &TemplateCreateRequest,
    backend: &str,
) -> Result<(), TemplateOutcomeError> {
    let shape = pool_shape(request, backend).ok_or_else(rejected)?;
    let identity = &request.input.generation;
    super::admit_common_metadata(pod, secret, request)?;
    // The required key is absent. Kubernetes 2.33.0 treats an empty data
    // map as computed; the bundled template uses a fixed nonsecret marker
    // instead. Known empty maps from retained templates remain safe.
    let data = secret.known("/data")?;
    if data != &json!({}) && data != &json!({"runner_backend":shape.backend}) {
        return Err(rejected());
    }

    super::admit_common_pod(pod, image)?;
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
    // Terraform provider schema uses a string; the Kubernetes API uses an integer.
    let user = pod.known(&format!("{security}/run_as_user"))?;
    if user != &json!(shape.run_as_user.to_string()) && user != &json!(shape.run_as_user) {
        return Err(rejected());
    }
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
    pod.exact(&format!("{runner}/command"), &json!(shape.command))?;
    pod.exact(&format!("{runner}/args"), &json!(shape.args))?;
    if let Some(directory) = shape.working_dir {
        pod.exact(&format!("{runner}/working_dir"), &json!(directory))?;
    }
    for field in ["env", "env_from", "lifecycle", "volume_device"] {
        pod.empty(&format!("{runner}/{field}"))?;
    }
    let expected = 1 + usize::from(shape.scratch.is_some());
    let mounts = count(pod, &format!("{runner}/volume_mount"))?;
    let volumes = count(pod, "/spec/0/volume")?;
    if mounts != expected || volumes != expected {
        return Err(rejected());
    }
    let mut gate = None;
    let mut scratch = None;
    for index in 0..mounts {
        let mount = format!("{runner}/volume_mount/{index}");
        let path = pod.known(&format!("{mount}/mount_path"))?;
        let name = pod
            .known(&format!("{mount}/name"))?
            .as_str()
            .filter(|name| !name.is_empty())
            .ok_or_else(rejected)?;
        if path == shape.mount_path && gate.is_none() {
            pod.exact(&format!("{mount}/sub_path"), &json!(shape.key))?;
            pod.exact(&format!("{mount}/read_only"), &json!(true))?;
            gate = Some(name);
        } else if shape.scratch.is_some_and(|scratch| path == scratch) && scratch.is_none() {
            for (field, allowed) in [("sub_path", ""), ("mount_propagation", "None")] {
                let value = pod.known(&format!("{mount}/{field}"))?;
                if !value.is_null() && value != allowed {
                    return Err(rejected());
                }
            }
            pod.exact(&format!("{mount}/read_only"), &json!(false))?;
            scratch = Some(name);
        } else {
            return Err(rejected());
        }
    }
    let gate = gate.ok_or_else(rejected)?;
    for index in 0..volumes {
        let volume = format!("/spec/0/volume/{index}");
        let name = pod.known(&format!("{volume}/name"))?;
        if name == gate {
            super::admit_secret_volume(pod, &volume, &identity.generation_name, shape.key)?;
        } else if scratch.is_some_and(|scratch| name == scratch) {
            empty_dir_volume(pod, &volume)?;
        } else {
            return Err(rejected());
        }
    }
    Ok(())
}

fn count(pod: &Planned<'_>, pointer: &str) -> Result<usize, TemplateOutcomeError> {
    pod.value(pointer)?
        .as_array()
        .map(Vec::len)
        .ok_or_else(rejected)
}

// A writable scratch directory only: node disk, never memory-backed.
fn empty_dir_volume(pod: &Planned<'_>, volume: &str) -> Result<(), TemplateOutcomeError> {
    pod.one(&format!("{volume}/empty_dir"))?;
    let medium = pod.known(&format!("{volume}/empty_dir/0/medium"))?;
    if !medium.is_null() && medium != "" {
        return Err(rejected());
    }
    super::only_volume_field(pod, volume, "empty_dir")
}
