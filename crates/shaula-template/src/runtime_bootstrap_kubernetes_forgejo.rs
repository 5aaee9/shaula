//! Forgejo `one-job` in the official image; the token reaches the runner
//! only as the read-only `/data/.forgejo-token` Secret item.
use super::pool::PoolPod;
use shaula_core::ports::TemplateCreateRequest;

pub(in crate::runtime::bootstrap) fn shape(request: &TemplateCreateRequest) -> Option<PoolPod<'_>> {
    let material = request.forgejo_bootstrap.as_ref()?;
    Some(PoolPod {
        backend: "forgejo",
        command: ["/usr/bin/dumb-init", "--", "/bin/forgejo-runner"],
        args: material.one_job_args(),
        run_as_user: 1000,
        working_dir: None,
        key: "token",
        mount_path: "/data/.forgejo-token",
        scratch: None,
        secret: material.token(),
        payload: material.token().to_owned(),
    })
}

#[cfg(test)]
#[path = "runtime_bootstrap_kubernetes_forgejo_tests.rs"]
mod tests;
