//! Validate the real saved plan before Create can launch an early Listener.
//! Sensitive plan values never appear in errors, logs or operation captures.
use super::{selected_image, TemplateCreateRequest, TemplateOutcomeError};
use serde_json::Value;
use shaula_core::template::ProfileManifest;

#[path = "runtime_bootstrap_plan_docker.rs"]
mod docker;
#[path = "runtime_bootstrap_plan_kubernetes.rs"]
mod kubernetes;

fn rejected() -> TemplateOutcomeError {
    TemplateOutcomeError::PlanFailed {
        phase: "create.plan".into(),
    }
}

pub(in crate::runtime) fn admit_plan(
    manifest: &ProfileManifest,
    request: &TemplateCreateRequest,
    plan: &Value,
) -> Result<(), TemplateOutcomeError> {
    if manifest.container_bootstrap_contract.is_none() {
        return Ok(());
    }
    let image = selected_image(manifest, request).map_err(|_| rejected())?;
    let changes = plan["resource_changes"].as_array().ok_or_else(rejected)?;
    let managed: Vec<_> = changes
        .iter()
        .filter(|change| change["mode"] == "managed")
        .collect();
    // No resource-local hooks may release the gate behind the Runtime's back.
    let root_configuration = &plan["configuration"]["root_module"];
    if !root_configuration.is_object() {
        return Err(rejected());
    }
    let configuration = modules(root_configuration)?;
    if configuration.iter().any(|resource| {
        resource
            .get("provisioners")
            .is_some_and(|value| !empty(value))
    }) {
        return Err(rejected());
    }
    match manifest.platform.as_str() {
        "docker" if managed.len() == 1 => {
            let runner = Planned::new(
                managed[0],
                "docker_container",
                "registry.terraform.io/kreuzwerker/docker",
            )?;
            docker::admit(
                &runner,
                plan,
                &image,
                request,
                manifest.runner_backend.as_str(),
            )
        }
        "kubernetes" if managed.len() == 2 => {
            let pod = managed
                .iter()
                .find(|resource| resource["type"] == "kubernetes_pod_v1")
                .ok_or_else(rejected)?;
            let secret = managed
                .iter()
                .find(|resource| resource["type"] == "kubernetes_secret_v1")
                .ok_or_else(rejected)?;
            let provider = "registry.terraform.io/hashicorp/kubernetes";
            kubernetes::admit(
                &Planned::new(pod, "kubernetes_pod_v1", provider)?,
                &Planned::new(secret, "kubernetes_secret_v1", provider)?,
                &image,
                request,
                manifest.runner_backend.as_str(),
            )
        }
        _ => Err(rejected()),
    }
}

struct Planned<'a> {
    after: &'a Value,
    unknown: &'a Value,
}

impl<'a> Planned<'a> {
    fn new(resource: &'a Value, kind: &str, provider: &str) -> Result<Self, TemplateOutcomeError> {
        if resource["type"] != kind
            || resource["provider_name"] != provider
            || resource["change"]["actions"] != serde_json::json!(["create"])
            || !resource["change"]["after"].is_object()
            || !resource["change"]["after_unknown"].is_object()
        {
            return Err(rejected());
        }
        Ok(Self {
            after: &resource["change"]["after"],
            unknown: &resource["change"]["after_unknown"],
        })
    }

    fn value(&self, pointer: &str) -> Result<&'a Value, TemplateOutcomeError> {
        let mut unknown = self.unknown;
        for segment in pointer.strip_prefix('/').ok_or_else(rejected)?.split('/') {
            if unknown == true {
                return Err(rejected());
            }
            let segment = segment.replace("~1", "/").replace("~0", "~");
            unknown = match unknown {
                Value::Array(values) => segment
                    .parse::<usize>()
                    .ok()
                    .and_then(|index| values.get(index))
                    .unwrap_or(&Value::Null),
                Value::Object(values) => values.get(&segment).unwrap_or(&Value::Null),
                _ => &Value::Null,
            };
        }
        if unknown == true {
            return Err(rejected());
        }
        Ok(self.after.pointer(pointer).unwrap_or(&Value::Null))
    }

    fn known(&self, pointer: &str) -> Result<&'a Value, TemplateOutcomeError> {
        let value = self.value(pointer)?;
        if has_unknown(self.unknown.pointer(pointer).unwrap_or(&Value::Null), 0)? {
            return Err(rejected());
        }
        Ok(value)
    }

    fn exact(&self, pointer: &str, expected: &Value) -> Result<(), TemplateOutcomeError> {
        if self.known(pointer)? != expected {
            return Err(rejected());
        }
        Ok(())
    }

    fn empty(&self, pointer: &str) -> Result<(), TemplateOutcomeError> {
        if !empty(self.known(pointer)?) {
            return Err(rejected());
        }
        Ok(())
    }

    fn one(&self, pointer: &str) -> Result<(), TemplateOutcomeError> {
        if self
            .value(pointer)?
            .as_array()
            .is_none_or(|values| values.len() != 1)
        {
            return Err(rejected());
        }
        Ok(())
    }
}

fn empty(value: &Value) -> bool {
    value.is_null()
        || value.as_array().is_some_and(Vec::is_empty)
        || value.as_object().is_some_and(serde_json::Map::is_empty)
}

fn has_unknown(value: &Value, depth: u8) -> Result<bool, TemplateOutcomeError> {
    if depth > 32 {
        return Err(rejected());
    }
    match value {
        Value::Bool(value) => Ok(*value),
        Value::Array(values) => {
            for value in values {
                if has_unknown(value, depth + 1)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        Value::Object(values) => {
            for value in values.values() {
                if has_unknown(value, depth + 1)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        Value::Null => Ok(false),
        _ => Err(rejected()),
    }
}

fn modules(root: &Value) -> Result<Vec<&Value>, TemplateOutcomeError> {
    fn walk<'a>(
        module: &'a Value,
        depth: u8,
        values: &mut Vec<&'a Value>,
    ) -> Result<(), TemplateOutcomeError> {
        if depth > 32 || values.len() > 128 {
            return Err(rejected());
        }
        if let Some(resources) = module.get("resources") {
            values.extend(resources.as_array().ok_or_else(rejected)?);
        }
        if let Some(children) = module.get("child_modules") {
            for child in children.as_array().ok_or_else(rejected)? {
                walk(child, depth + 1, values)?;
            }
        }
        // The configuration representation uses module_calls instead.
        if let Some(calls) = module.get("module_calls") {
            for call in calls.as_object().ok_or_else(rejected)?.values() {
                walk(&call["module"], depth + 1, values)?;
            }
        }
        if values.len() > 128 {
            return Err(rejected());
        }
        Ok(())
    }
    let mut values = Vec::new();
    walk(root, 0, &mut values)?;
    Ok(values)
}

#[cfg(test)]
#[path = "runtime_bootstrap_plan_tests.rs"]
mod tests;
