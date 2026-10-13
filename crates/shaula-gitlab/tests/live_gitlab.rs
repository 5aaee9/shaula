//! Adapter E2E against a disposable GitLab. Run through
//! `scripts/gitlab-pool-spike/adapter-e2e.mjs`; skipped (ignored) otherwise.
//! Writes a JSON artifact of every observation to the fixture's path.

mod support;

use reqwest::Method;
use serde_json::{json, Value};
use shaula_core::gitlab::{
    ownership_note, FleetGitlabSection, GitlabRegistrationRole, GitlabScope,
};
use shaula_core::ports::gitlab::{
    GitlabBootstrapMaterial, GitlabPoolPort, GitlabRemovalOutcome, GitlabRunnerSpec,
};
use shaula_core::ports::{AccessFailure, EffectOutcome};
use shaula_core::secret::SecretString;
use shaula_gitlab::GitlabClient;
use std::time::Duration;
use support::{read_secret, until, Admin, E2eConfig, Engine, TestResult};

const FLEET: &str = "e2e";
const MINUTE: Duration = Duration::from_secs(60);

fn section(config: &E2eConfig) -> FleetGitlabSection {
    FleetGitlabSection {
        instance_url: config.url.clone(),
        scope: GitlabScope::Group {
            path: config.group_path.clone(),
        },
        auth_profile_ref: "gitlab".into(),
        runner_name_prefix: "shaula-e2e".into(),
        tags: vec![config.tag.clone()],
        run_untagged: false,
    }
}

fn spec(
    section: &FleetGitlabSection,
    role: GitlabRegistrationRole,
    name: &str,
) -> GitlabRunnerSpec {
    GitlabRunnerSpec {
        description: name.into(),
        maintenance_note: ownership_note(FLEET, role, name),
        role,
        matching: section.matching(),
    }
}

async fn register(
    port: &dyn GitlabPoolPort,
    spec: &GitlabRunnerSpec,
) -> TestResult<(u64, SecretString)> {
    match port.register_runner(spec).await {
        Ok(EffectOutcome::Definite(registration)) => Ok((registration.id, registration.token)),
        other => Err(format!("registration was not definite: {other:?}").into()),
    }
}

#[tokio::test]
#[ignore = "needs the disposable GitLab from scripts/gitlab-pool-spike/adapter-e2e.mjs"]
async fn adapter_drives_a_full_generation_against_real_gitlab() -> TestResult {
    let Some(config) = E2eConfig::load()? else {
        return Err("SHAULA_GITLAB_E2E is not set".into());
    };
    let admin = Admin::new(&config)?;
    let mut engine = Engine::new(&config);
    let fleet = section(&config);
    fleet.validate()?;
    let operator = SecretString::new(read_secret(&config.operator_token_file)?);
    let client = GitlabClient::new(fleet.target(), operator.clone())?
        .with_expected_scope_id(config.group_id)?;
    // Every call goes through the core port, as the daemon will use it.
    let port: &dyn GitlabPoolPort = &client;
    let mut report = serde_json::Map::new();

    // Auth probe: version, pinned scope id, token scopes, inventory.
    let probe = port.auth_probe().await.map_err(|e| e.summary())?;
    assert_eq!(probe.target_id, config.group_id);
    assert_eq!(probe.runner_count, 0);
    report.insert(
        "probe".into(),
        json!({ "serverVersion": probe.server_version, "tokenExpiresAt": probe.token_expires_at }),
    );

    // Failure mode 4: a path re-pointed at another id fails closed.
    let repointed = GitlabClient::new(fleet.target(), operator.clone())?
        .with_expected_scope_id(config.group_id + 1_000)?;
    let repointed: &dyn GitlabPoolPort = &repointed;
    let repointed = repointed.list_runners("shaula-e2e").await;
    assert_eq!(repointed, Err(AccessFailure::PermissionDenied));

    // Anchor: paused, token dropped immediately.
    let anchor_spec = spec(&fleet, GitlabRegistrationRole::Anchor, "shaula-e2e-anchor");
    let (anchor, anchor_token) = register(port, &anchor_spec).await?;
    drop(anchor_token);
    let listed = port
        .list_runners("shaula-e2e")
        .await
        .map_err(|e| e.summary())?;
    assert_eq!(listed.len(), 1);
    assert_eq!(
        listed[0].maintenance_note.as_deref(),
        Some(anchor_spec.maintenance_note.as_str())
    );
    assert!(listed[0].paused);
    let idle = port.anchor_demand(anchor).await.map_err(|e| e.summary())?;
    assert_eq!(idle.proven_for(&fleet.matching()), Some(0));

    let project = config.project_id;
    admin
        .call(
            Method::POST,
            &format!("/projects/{project}/pipeline"),
            json!({ "ref": "main" }),
        )
        .await?;
    let pending = until("compatible pending job", MINUTE, || async {
        let demand = port.anchor_demand(anchor).await.map_err(|e| e.summary())?;
        Ok(demand
            .proven_for(&fleet.matching())
            .filter(|count| *count > 0))
    })
    .await?;
    assert_eq!(pending, 1, "the gpu-tagged job is not compatible");

    // Failure mode 7: an unpaused (drifted) anchor no longer proves demand.
    admin
        .call(
            Method::PUT,
            &format!("/runners/{anchor}"),
            json!({ "paused": false }),
        )
        .await?;
    let drifted = port.anchor_demand(anchor).await.map_err(|e| e.summary())?;
    assert_eq!(drifted.proven_for(&fleet.matching()), None);
    assert_eq!(
        port.pause_runner(anchor).await.map_err(|e| e.summary())?,
        GitlabRemovalOutcome::Done
    );
    report.insert(
        "demand".into(),
        json!({ "idle": 0, "pending": pending, "unpausedAnchorProven": false }),
    );

    // Generation: register, deliver config.toml, run-single picks the job.
    let name = "shaula-e2e-gen-1";
    let (generation, token) = register(
        port,
        &spec(&fleet, GitlabRegistrationRole::Generation, name),
    )
    .await?;
    let material =
        GitlabBootstrapMaterial::new(&config.internal_url, generation, name, token.clone())?;
    let before = port
        .get_runner(generation)
        .await
        .map_err(|e| e.summary())?
        .ok_or("generation missing")?;
    assert_eq!(before.status, "never_contacted");
    assert!(!before.paused);
    let container = engine.start_runner(
        &config,
        name,
        &material.run_single_args("/etc/gitlab-runner/config.toml", 120),
        &material.config_toml(),
    )?;
    let jobs = until("job association", 3 * MINUTE, || async {
        let jobs = port
            .running_jobs(generation)
            .await
            .map_err(|e| e.summary())?;
        Ok(jobs.filter(|jobs| !jobs.is_empty()))
    })
    .await?;
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].project_id, project);
    assert_eq!(jobs[0].name, "match");
    let job_id = jobs[0].id;

    // Pause right after acquisition; the running job is not interrupted.
    assert_eq!(
        port.pause_runner(generation)
            .await
            .map_err(|e| e.summary())?,
        GitlabRemovalOutcome::Done
    );
    let busy = port
        .get_runner(generation)
        .await
        .map_err(|e| e.summary())?
        .ok_or("generation missing")?;
    assert!(busy.paused && busy.is_active());

    let exit = until("runner exit", 3 * MINUTE, || async {
        engine.exit_code(&container)
    })
    .await?;
    assert_eq!(exit, 0);
    let job: Value = admin
        .call(
            Method::GET,
            &format!("/projects/{project}/jobs/{job_id}"),
            Value::Null,
        )
        .await?;
    assert_eq!(job["status"], "success");
    let after = port
        .running_jobs(generation)
        .await
        .map_err(|e| e.summary())?;
    assert_eq!(after, Some(Vec::new()));
    // job_execution_status lags the job's terminal state; record the lag
    // and the values seen so §4.5 can rely on the observed behavior.
    let exited_at = std::time::Instant::now();
    let observed = std::sync::Mutex::new(Vec::new());
    let settled = until("registration idle after exit", 3 * MINUTE, || {
        let observed = &observed;
        async move {
            let runner = port
                .get_runner(generation)
                .await
                .map_err(|e| e.summary())?
                .ok_or("generation missing")?;
            let seen = (runner.status.clone(), runner.job_execution_status.clone());
            let mut observed = observed.lock().map_err(|_| "observation lock poisoned")?;
            if observed.last() != Some(&seen) {
                eprintln!("registration after exit: {seen:?}");
                observed.push(seen);
            }
            Ok((runner.is_known() && !runner.is_active()).then_some(runner))
        }
    })
    .await?;
    let idle_lag_secs = exited_at.elapsed().as_secs();

    // Leak scan: the token never reaches argv, env, logs or the job trace.
    // `run-single` streams job output to GitLab, not to the container log.
    let inspect = engine.run(&["inspect", &container])?;
    let logs = engine.run(&["logs", &container])?;
    let trace = admin
        .text(&format!("/projects/{project}/jobs/{job_id}/trace"))
        .await?;
    assert!(!inspect.contains(token.expose()));
    assert!(!logs.contains(token.expose()));
    assert!(!trace.contains(token.expose()));
    assert!(trace.contains("TASK_EXECUTED"));
    report.insert(
        "generation".into(),
        json!({
            "neverContactedBeforeStart": true,
            "associatedJob": { "name": jobs[0].name, "projectPath": jobs[0].project_path },
            "pausedWhileBusy": true,
            "exitCode": exit,
            "jobStatus": job["status"],
            "runningAfterExit": 0,
            "statusAfterExit": settled.status,
            "executionStatusesAfterExit": observed.into_inner().map_err(|_| "observation lock poisoned")?,
            "idleLagSeconds": idle_lag_secs,
            "traceHasTaskOutput": true,
            "tokenInInspectLogsOrTrace": false,
        }),
    );

    // Removal is idempotent and leaves the anchor's demand unaffected.
    assert_eq!(
        port.delete_runner(generation)
            .await
            .map_err(|e| e.summary())?,
        GitlabRemovalOutcome::Done
    );
    assert_eq!(
        port.delete_runner(generation)
            .await
            .map_err(|e| e.summary())?,
        GitlabRemovalOutcome::AlreadyAbsent
    );
    assert_eq!(
        port.get_runner(generation).await.map_err(|e| e.summary())?,
        None
    );
    assert_eq!(
        port.running_jobs(generation)
            .await
            .map_err(|e| e.summary())?,
        None
    );
    assert_eq!(
        port.pause_runner(generation)
            .await
            .map_err(|e| e.summary())?,
        GitlabRemovalOutcome::AlreadyAbsent
    );
    assert_eq!(
        port.delete_runner(anchor).await.map_err(|e| e.summary())?,
        GitlabRemovalOutcome::Done
    );
    assert!(port
        .list_runners("shaula-e2e")
        .await
        .map_err(|e| e.summary())?
        .is_empty());
    report.insert("removal".into(), json!({ "generation": "Done then AlreadyAbsent", "anchor": "Done", "inventoryEmpty": true }));

    std::fs::write(
        &config.artifact,
        serde_json::to_vec_pretty(&Value::Object(report))?,
    )?;
    Ok(())
}
