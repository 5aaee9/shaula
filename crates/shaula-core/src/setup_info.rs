//! Independent, read-only Runner setup-log delivery capabilities.
use crate::{error::CoreResult, template::SetupInfoDescriptor};
use async_trait::async_trait;
use serde::Deserialize;
use std::net::SocketAddr;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetupInfoConfig {
    #[serde(default = "default_listen")]
    pub listen: SocketAddr,
    pub advertised_origin: String,
    #[serde(default = "default_ttl")]
    pub capability_ttl_seconds: u32,
    #[serde(default = "default_wait")]
    pub wait_seconds: u32,
    #[serde(default = "default_rate")]
    pub requests_per_second: u32,
    #[serde(default = "default_burst")]
    pub burst: u32,
    #[serde(default = "default_concurrency")]
    pub max_concurrent_requests: usize,
}

fn default_listen() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 9091))
}
fn default_ttl() -> u32 {
    3600
}
fn default_wait() -> u32 {
    60
}
fn default_rate() -> u32 {
    2
}
fn default_burst() -> u32 {
    4
}
fn default_concurrency() -> usize {
    64
}

impl SetupInfoConfig {
    pub fn validate(&self) -> Result<(), String> {
        let origin =
            url::Url::parse(&self.advertised_origin).map_err(|_| "setup_info origin invalid")?;
        if !self.listen.ip().is_loopback()
            || origin.scheme() != "https"
            || origin.host_str().is_none()
            || !origin.username().is_empty()
            || origin.password().is_some()
            || origin.query().is_some()
            || origin.fragment().is_some()
            || origin.path() != "/"
            || self.advertised_origin.len() > 1024
            || self.advertised_origin.chars().any(char::is_whitespace)
            || origin.port() == Some(0)
            || !(1..=86400).contains(&self.capability_ttl_seconds)
            || !(1..=300).contains(&self.wait_seconds)
            || !(1..=100).contains(&self.requests_per_second)
            || !(1..=1000).contains(&self.burst)
            || !(1..=1024).contains(&self.max_concurrent_requests)
        {
            return Err(
                "setup_info requires a loopback listener, HTTPS origin and bounded limits".into(),
            );
        }
        Ok(())
    }
}

#[async_trait]
pub trait SetupInfoIssuer: Send + Sync {
    /// Called once before freezing fresh v2 input. `now` is Unix SECONDS, unlike lifecycle clocks.
    /// Never reissues on a read/recovery path.
    async fn issue(&self, generation_id: &str, now: i64) -> CoreResult<SetupInfoDescriptor>;
}

#[async_trait]
pub trait SetupInfoAuthorizer: Send + Sync {
    /// `now` is Unix seconds, matching descriptor expiry and HTTP wall-clock authentication.
    async fn authorize(&self, generation_id: &str, capability: &str, now: i64) -> CoreResult<bool>;
}
