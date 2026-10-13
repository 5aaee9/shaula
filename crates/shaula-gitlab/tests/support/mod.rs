//! Live-GitLab fixture plumbing for `live_gitlab.rs`. The disposable server is
//! provisioned by `scripts/gitlab-pool-spike/adapter-e2e.mjs`.

use serde::Deserialize;
use serde_json::Value;
use std::future::Future;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::process::Command;
use std::time::{Duration, Instant};

pub type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

#[derive(Deserialize)]
pub struct E2eConfig {
    pub url: String,
    pub internal_url: String,
    pub group_path: String,
    pub group_id: u64,
    pub project_id: u64,
    pub tag: String,
    pub operator_token_file: String,
    pub admin_token_file: String,
    pub engine: String,
    pub network: String,
    pub runner_image: String,
    pub work_dir: String,
    pub artifact: String,
}

impl E2eConfig {
    /// `None` when the live fixture is not provisioned.
    pub fn load() -> TestResult<Option<Self>> {
        let Ok(path) = std::env::var("SHAULA_GITLAB_E2E") else {
            return Ok(None);
        };
        Ok(Some(serde_json::from_slice(&std::fs::read(path)?)?))
    }
}

pub fn read_secret(path: &str) -> TestResult<String> {
    Ok(std::fs::read_to_string(path)?.trim().to_owned())
}

/// Fixture-only admin calls (pipelines, job state, out-of-band runner edits).
pub struct Admin {
    client: reqwest::Client,
    url: String,
    token: String,
}

impl Admin {
    pub fn new(config: &E2eConfig) -> TestResult<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(15))
                .build()?,
            url: format!("{}/api/v4", config.url),
            token: read_secret(&config.admin_token_file)?,
        })
    }

    pub async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Value,
    ) -> TestResult<Value> {
        let response = self
            .client
            .request(method, format!("{}{path}", self.url))
            .header("PRIVATE-TOKEN", &self.token)
            .json(&body)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("admin {path}: HTTP {status}").into());
        }
        Ok(response.json().await?)
    }

    /// Plain-text reads such as a job trace.
    pub async fn text(&self, path: &str) -> TestResult<String> {
        let response = self
            .client
            .get(format!("{}{path}", self.url))
            .header("PRIVATE-TOKEN", &self.token)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("admin {path}: HTTP {status}").into());
        }
        Ok(response.text().await?)
    }
}

pub async fn until<T, F, Fut>(description: &str, timeout: Duration, mut probe: F) -> TestResult<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = TestResult<Option<T>>>,
{
    let started = Instant::now();
    loop {
        if let Some(value) = probe().await? {
            return Ok(value);
        }
        if started.elapsed() > timeout {
            return Err(format!("timed out waiting for {description}").into());
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// Containers created here are removed by ID on drop; never by name or prune.
pub struct Engine {
    engine: String,
    containers: Vec<String>,
}

impl Engine {
    pub fn new(config: &E2eConfig) -> Self {
        Self {
            engine: config.engine.clone(),
            containers: Vec::new(),
        }
    }

    pub fn run(&self, args: &[&str]) -> TestResult<String> {
        let output = Command::new(&self.engine).args(args).output()?;
        if !output.status.success() {
            // Output may contain credentials; report only the subcommand.
            return Err(format!("{} {} failed", self.engine, args[0]).into());
        }
        let mut text = String::from_utf8(output.stdout)?;
        text.push_str(&String::from_utf8(output.stderr)?);
        Ok(text.trim().to_owned())
    }

    /// Starts `run-single`; the token reaches the container only as a copied file.
    pub fn start_runner(
        &mut self,
        config: &E2eConfig,
        name: &str,
        args: &[String],
        config_toml: &str,
    ) -> TestResult<String> {
        let mut create = vec!["create", "--network", &config.network, "--restart", "no"];
        create.push(&config.runner_image);
        create.extend(args.iter().map(String::as_str));
        let id = self.run(&create)?;
        if id.len() != 64 || !id.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err("unexpected container id".into());
        }
        self.containers.push(id.clone());
        let path = format!("{}/{name}.toml", config.work_dir);
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?
            .write_all(config_toml.as_bytes())?;
        let copied = self.run(&["cp", &path, &format!("{id}:/etc/gitlab-runner/config.toml")]);
        std::fs::remove_file(&path)?;
        copied?;
        self.run(&["start", &id])?;
        Ok(id)
    }

    /// `Some(exit_code)` once the container stopped.
    pub fn exit_code(&self, id: &str) -> TestResult<Option<i64>> {
        let state = self.run(&[
            "inspect",
            "--format",
            "{{.State.Running}} {{.State.ExitCode}}",
            id,
        ])?;
        match state.split_once(' ') {
            Some(("false", code)) => Ok(Some(code.parse()?)),
            _ => Ok(None),
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        for id in self.containers.drain(..).rev() {
            let _ = Command::new(&self.engine)
                .args(["rm", "--force", "--volumes", &id])
                .output();
        }
    }
}
