//! GitLab `run-single` in the official image (spec 0043 §5). The token
//! reaches the runner only inside the read-only `config.toml` Secret item,
//! mounted into a writable Generation-owned emptyDir home.
use super::pool::PoolPod;
use shaula_core::{gitlab::RUNNER_WAIT_TIMEOUT_SECS, ports::TemplateCreateRequest};

const HOME: &str = "/home/gitlab-runner";
const CONFIG_PATH: &str = "/home/gitlab-runner/config.toml";

pub(in crate::runtime::bootstrap) fn shape(request: &TemplateCreateRequest) -> Option<PoolPod<'_>> {
    let material = request.gitlab_bootstrap.as_ref()?;
    // The non-secret Terraform input must name the exact same registration.
    if request.input.gitlab.as_ref() != Some(&material.identity()) {
        return None;
    }
    Some(PoolPod {
        backend: "gitlab",
        command: ["/usr/bin/dumb-init", "--", "/usr/bin/gitlab-runner"],
        args: material.run_single_args(CONFIG_PATH, RUNNER_WAIT_TIMEOUT_SECS),
        // The image defaults to root; 999 is its packaged gitlab-runner user.
        run_as_user: 999,
        working_dir: Some(HOME),
        key: "config.toml",
        mount_path: CONFIG_PATH,
        // run-single writes .runner_system_id beside its config and the shell
        // executor builds below the working directory.
        scratch: Some(HOME),
        secret: material.token(),
        payload: material.config_toml(),
    })
}
