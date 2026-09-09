use super::{modules, rejected, Planned, TemplateCreateRequest, TemplateOutcomeError};
use serde_json::{json, Value};

pub(super) fn admit(
    runner: &Planned<'_>,
    plan: &Value,
    image: &str,
    request: &TemplateCreateRequest,
) -> Result<(), TemplateOutcomeError> {
    let identity = &request.input.generation;
    let short: String = identity.id.chars().take(24).collect();
    runner.exact("/start", &json!(false))?;
    runner.exact("/must_run", &json!(false))?;
    runner.exact("/rm", &json!(false))?;
    runner.exact("/restart", &json!("no"))?;
    runner.exact("/wait", &json!(false))?;
    runner.exact(
        "/name",
        &json!(format!("shaula-{}-{short}", identity.fleet_key)),
    )?;
    runner.exact("/command", &json!([super::super::LISTENER, "run"]))?;
    runner.exact(
        "/env",
        &json!([format!(
            "ACTIONS_RUNNER_INPUT_JITCONFIG={}",
            request.input.jit_config
        )]),
    )?;
    for pointer in [
        "/upload",
        "/mounts",
        "/volumes",
        "/devices",
        "/capabilities",
    ] {
        runner.empty(pointer)?;
    }
    let privileged = runner.known("/privileged")?;
    if !privileged.is_null() && privileged != false {
        return Err(rejected());
    }
    let labels = runner.known("/labels")?.as_array().ok_or_else(rejected)?;
    if labels.len() != 2
        || !labels.contains(&json!({"label":"shaula.fleet","value":identity.fleet_key}))
        || !labels.contains(&json!({"label":"shaula.generation","value":identity.id}))
    {
        return Err(rejected());
    }
    let actual = runner.known("/image")?.as_str().ok_or_else(rejected)?;
    if actual != image && !resolved_image(plan, image, actual)? {
        return Err(rejected());
    }
    // entrypoint/user are inherited, computed fields in provider 3.0.2. Their
    // live values are checked while start=false still prevents execution.
    Ok(())
}

fn resolved_image(plan: &Value, image: &str, id: &str) -> Result<bool, TemplateOutcomeError> {
    // Terraform 1.9.8 places already-read data sources in prior_state, not
    // necessarily planned_values or resource_changes. Both are saved-plan
    // evidence; never infer Docker's image/config ID from the manifest digest.
    let mut resources = modules(&plan["planned_values"]["root_module"])?;
    resources.extend(modules(&plan["prior_state"]["values"]["root_module"])?);
    Ok(resources.iter().any(|resource| {
        resource["mode"] == "data"
            && resource["type"] == "docker_image"
            && resource["provider_name"] == "registry.terraform.io/kreuzwerker/docker"
            && resource["values"]["name"] == image
            && resource["values"]["id"] == id
    }))
}
