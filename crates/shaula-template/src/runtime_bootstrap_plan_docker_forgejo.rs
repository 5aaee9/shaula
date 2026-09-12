use super::{rejected, Planned, TemplateCreateRequest, TemplateOutcomeError};
use serde_json::{json, Value};

pub(super) fn admit(
    runner: &Planned<'_>,
    plan: &Value,
    image: &str,
    request: &TemplateCreateRequest,
) -> Result<(), TemplateOutcomeError> {
    let material = request.forgejo_bootstrap.as_ref().ok_or_else(rejected)?;
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
    runner.exact("/command", &command(material))?;
    runner.exact("/entrypoint", &json!(["/usr/bin/dumb-init"]))?;
    runner.exact("/working_dir", &json!("/data"))?;
    runner.exact("/remove_volumes", &json!(true))?;
    runner.exact("/env", &json!([]))?;
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
    if actual != image && !super::resolved_image(plan, image, actual)? {
        return Err(rejected());
    }
    Ok(())
}

fn command(material: &shaula_core::ports::forgejo::ForgejoBootstrapMaterial) -> Value {
    let mut command = vec!["/bin/forgejo-runner".to_string()];
    command.extend(material.one_job_args());
    json!(command)
}
