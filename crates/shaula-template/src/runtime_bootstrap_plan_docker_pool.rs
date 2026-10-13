//! Saved-plan admission shared by the Pool backends (Forgejo, GitLab). The
//! backend fixes the launch command, working directory and user.
use super::{rejected, Planned, TemplateCreateRequest, TemplateOutcomeError};
use serde_json::{json, Value};

use shaula_core::gitlab::RUNNER_WAIT_TIMEOUT_SECS;

pub(super) fn admit(
    runner: &Planned<'_>,
    plan: &Value,
    image: &str,
    request: &TemplateCreateRequest,
    backend: &str,
) -> Result<(), TemplateOutcomeError> {
    let (command, working_dir, user) = match backend {
        "forgejo" => {
            let material = request.forgejo_bootstrap.as_ref().ok_or_else(rejected)?;
            let mut command = vec!["/bin/forgejo-runner".to_string()];
            command.extend(material.one_job_args());
            (command, "/data", None)
        }
        "gitlab" => {
            let material = request.gitlab_bootstrap.as_ref().ok_or_else(rejected)?;
            if request.input.gitlab.as_ref() != Some(&material.identity()) {
                return Err(rejected());
            }
            let mut command = vec!["/usr/bin/gitlab-runner".to_string()];
            command.extend(
                material
                    .run_single_args("/home/gitlab-runner/config.toml", RUNNER_WAIT_TIMEOUT_SECS),
            );
            (command, "/home/gitlab-runner", Some("gitlab-runner"))
        }
        _ => return Err(rejected()),
    };
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
    runner.exact("/command", &json!(command))?;
    runner.exact("/entrypoint", &json!(["/usr/bin/dumb-init"]))?;
    runner.exact("/working_dir", &json!(working_dir))?;
    runner.exact("/remove_volumes", &json!(true))?;
    // The official GitLab image defaults to root; its user is pinned in the
    // plan here and re-proved on the stopped container.
    if let Some(user) = user {
        runner.exact("/user", &json!(user))?;
    }
    // Docker 3.0.2 makes an empty env set computed. Keep a fixed, nonsecret
    // value so the saved plan proves the input instead of admitting unknowns.
    let env = runner.known("/env")?;
    if env != &json!([]) && env != &json!([format!("SHAULA_RUNNER_BACKEND={backend}")]) {
        return Err(rejected());
    }
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
