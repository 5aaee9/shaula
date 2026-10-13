//! Demand through the paused anchor, and the non-mutating auth probe.

use serde::Deserialize;
use shaula_core::gitlab::GitlabMatching;
use shaula_core::ports::gitlab::{GitlabAnchorDemand, GitlabAuthProbe};

use crate::client::GitlabClient;
use crate::error::GitlabError;
use crate::models::RawTokenSelf;

const REQUIRED_SCOPES: [&str; 3] = ["create_runner", "manage_runner", "read_api"];

#[derive(Deserialize)]
struct Envelope {
    data: Option<DemandData>,
    #[serde(default)]
    errors: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct DemandData {
    jobs: Option<JobCount>,
    runner: Option<AnchorRunner>,
}

#[derive(Deserialize)]
struct JobCount {
    count: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AnchorRunner {
    paused: bool,
    #[serde(default)]
    tag_list: Vec<String>,
    run_untagged: bool,
    user_permissions: Permissions,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Permissions {
    update_runner: bool,
}

impl GitlabClient {
    /// One request carries both the server-side compatible count and the
    /// evidence that the filter was authorized (spec 0043 §3).
    pub async fn anchor_demand(&self, anchor_id: u64) -> Result<GitlabAnchorDemand, GitlabError> {
        if anchor_id == 0 {
            return Err(GitlabError::Configuration(
                "anchor id must be positive".into(),
            ));
        }
        self.scope_id().await?;
        // The id is numeric, so the literal cannot inject GraphQL syntax.
        let query = format!(
            "{{ jobs(statuses: [PENDING], compatibleRunnerId: \"gid://gitlab/Ci::Runner/{anchor_id}\") {{ count }} \
             runner(id: \"gid://gitlab/Ci::Runner/{anchor_id}\") {{ paused tagList runUntagged userPermissions {{ updateRunner }} }} }}"
        );
        let response = self
            .graphql(self.url(&["api", "graphql"])?)
            .json(&serde_json::json!({ "query": query }))
            .send()
            .await
            .map_err(|error| GitlabError::unavailable(error.without_url().to_string()))?;
        let envelope: Envelope = self.decode(response).await?;
        if envelope.errors.is_some_and(|errors| !errors.is_null()) {
            return Err(GitlabError::unavailable("GraphQL demand query failed"));
        }
        let data = envelope
            .data
            .ok_or_else(|| GitlabError::InvalidResponse("GraphQL data missing".into()))?;
        let pending = data
            .jobs
            .ok_or_else(|| GitlabError::InvalidResponse("GraphQL jobs missing".into()))?
            .count;
        Ok(match data.runner {
            Some(runner) => {
                let mut tags = runner.tag_list;
                tags.sort();
                GitlabAnchorDemand {
                    pending,
                    anchor_present: true,
                    anchor_manageable: runner.user_permissions.update_runner,
                    anchor_paused: runner.paused,
                    anchor_matching: GitlabMatching {
                        tags,
                        run_untagged: runner.run_untagged,
                    },
                }
            }
            None => GitlabAnchorDemand {
                pending,
                anchor_present: false,
                anchor_manageable: false,
                anchor_paused: false,
                anchor_matching: GitlabMatching {
                    tags: Vec::new(),
                    run_untagged: false,
                },
            },
        })
    }

    /// Does not mutate the server: version, pinned scope identity, token
    /// scopes and a full inventory read are captured together.
    pub async fn probe_authentication(&self) -> Result<GitlabAuthProbe, GitlabError> {
        let server_version = self.server_version().await?;
        let target_id = self.scope_id().await?;
        let token: RawTokenSelf = self
            .get(&["api", "v4", "personal_access_tokens", "self"])
            .await?;
        if !token.active
            || token.revoked
            || !REQUIRED_SCOPES
                .iter()
                .all(|scope| token.scopes.iter().any(|granted| granted == scope))
        {
            return Err(GitlabError::PermissionDenied);
        }
        let runner_count = self.runner_count().await?;
        let elapsed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| GitlabError::unavailable("system clock is before Unix epoch"))?;
        let checked_at_unix_ms = i64::try_from(elapsed.as_millis())
            .map_err(|_| GitlabError::unavailable("system clock exceeds millisecond range"))?;
        Ok(GitlabAuthProbe {
            server_version,
            target_id,
            checked_at_unix_ms,
            valid_until_unix_ms: checked_at_unix_ms.saturating_add(60_000),
            runner_count,
            token_expires_at: token.expires_at,
        })
    }
}
