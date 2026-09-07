//! `GitHubAccessPort` implementation on top of the wire client. Only the
//! core-owned port surfaces; wire DTOs never escape this module.

use std::time::Duration;

use async_trait::async_trait;
use shaula_core::github::Label;
use shaula_core::ports::AuthContext;
use shaula_core::ports::EffectOutcome;
use shaula_core::ports::GitHubAccessPort;
use shaula_core::ports::LookupOutcome;
use shaula_core::ports::PollOutcome;
use shaula_core::ports::ScaleSetView;
use shaula_core::ports::SessionHandle;

use crate::client::ScalesetClient;
use crate::error::ActionsException;
use crate::error::ScalesetError;
use crate::wire;

/// Long-poll ceiling per call; the listener re-polls in its own loop.
pub(crate) const LONG_POLL_TIMEOUT: Duration = Duration::from_secs(70);

impl ScalesetClient {
    pub(crate) async fn auth_kind(&self) -> AuthContext {
        AuthContext {
            kind: self.auth_kind_inner().await,
        }
    }

    async fn auth_kind_inner(&self) -> shaula_core::auth::AuthKind {
        // The admin token manager owns the credential; expose only its kind.
        self.admin.credential_kind()
    }

    async fn parse_error(response: reqwest::Response) -> ScalesetError {
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let body = response.text().await.unwrap_or_default();
        if content_type.contains("application/json") {
            if let Ok(exception) = serde_json::from_str::<wire::ActionsException>(&body) {
                if !exception.type_name.is_empty() {
                    return ScalesetError::Service {
                        exception: crate::error::classify_exception(&exception.type_name),
                        status,
                        summary: crate::error::classify_exception(&exception.type_name)
                            .summary_fallback(status),
                    };
                }
            }
        }
        ScalesetError::Status {
            status,
            summary: "request failed".into(),
        }
    }

    async fn expect_ok(
        response: Result<reqwest::Response, ScalesetError>,
    ) -> Result<reqwest::Response, ScalesetError> {
        let response = response?;
        if response.status().is_success() {
            Ok(response)
        } else {
            Err(Self::parse_error(response).await)
        }
    }
}

impl ActionsException {
    fn summary_fallback(self, status: u16) -> String {
        match self {
            ActionsException::AgentExists => "runner already exists".into(),
            ActionsException::AgentNotFound => "runner not found".into(),
            ActionsException::JobStillRunning => "job still running".into(),
            ActionsException::MessageQueueTokenExpired => "message queue token expired".into(),
            ActionsException::Other => format!("service error status={status}"),
        }
    }
}

pub(crate) async fn definite_or_uncertain<T>(
    response: Result<reqwest::Response, ScalesetError>,
) -> Result<EffectOutcome<T>, ScalesetError>
where
    T: serde::de::DeserializeOwned,
{
    match response {
        Ok(response) if response.status().is_success() => match response.json::<T>().await {
            Ok(value) => Ok(EffectOutcome::Definite(value)),
            Err(e) => Err(ScalesetError::MalformedResponse {
                summary: format!("body invalid: {e}"),
            }),
        },
        Ok(response) => {
            // Definite service-side rejection.
            Err(ScalesetClient::parse_error(response).await)
        }
        Err(e) => match e {
            ScalesetError::RequestUncertain { summary } => Ok(EffectOutcome::Uncertain { summary }),
            other => Err(other),
        },
    }
}

#[async_trait]
impl GitHubAccessPort for ScalesetClient {
    async fn auth_context(&self) -> AuthContext {
        self.auth_kind().await
    }

    async fn resolve_runner_group(
        &self,
        identity: &shaula_core::github::ScaleSetIdentity,
    ) -> Result<i64, shaula_core::ports::AccessFailure> {
        let response = self
            .actions_service_request(
                reqwest::Method::GET,
                "_apis/runtime/runnergroups/",
                &[("groupName", identity.runner_group.clone())],
                None,
                crate::client::DEFAULT_REQUEST_TIMEOUT,
            )
            .await;
        let response = Self::expect_ok(response)
            .await
            .map_err(|e| e.to_access_failure())?;
        let list = response
            .json::<wire::RunnerGroupList>()
            .await
            .map_err(|e| ScalesetError::MalformedResponse {
                summary: format!("runner group body invalid: {e}"),
            })
            .map_err(|e| e.to_access_failure())?;
        // R9-12: the ACTUAL entries decide, never the enveloped `count` —
        // a mismatched or malformed list can never drive an index panic.
        match list.value.as_slice() {
            [one] => Ok(one.id),
            [] => Err(shaula_core::ports::AccessFailure::TargetHiddenOrNotFound),
            _ => Err(shaula_core::ports::AccessFailure::Unavailable {
                summary: "multiple runner groups with name".into(),
            }),
        }
    }

    async fn lookup_scale_set(
        &self,
        identity: &shaula_core::github::ScaleSetIdentity,
        runner_group_id: i64,
    ) -> Result<LookupOutcome, shaula_core::ports::AccessFailure> {
        let response = self
            .actions_service_request(
                reqwest::Method::GET,
                wire::SCALE_SET_ENDPOINT,
                &[
                    ("runnerGroupId", runner_group_id.to_string()),
                    ("name", identity.scale_set_name.clone()),
                ],
                None,
                crate::client::DEFAULT_REQUEST_TIMEOUT,
            )
            .await;
        let response = Self::expect_ok(response)
            .await
            .map_err(|e| e.to_access_failure())?;
        let list = response
            .json::<wire::RunnerScaleSetList>()
            .await
            .map_err(|e| ScalesetError::MalformedResponse {
                summary: format!("scale set list body invalid: {e}"),
            })
            .map_err(|e| e.to_access_failure())?;
        // R9-12: decide on the entries, never on the enveloped `count` —
        // an inconsistent envelope cannot drive an index panic.
        match list.value.as_slice() {
            [] => Ok(LookupOutcome::None),
            [one] => Ok(LookupOutcome::ExactlyOne(scale_set_view(one))),
            _ => Ok(LookupOutcome::Multiple),
        }
    }

    async fn create_scale_set(
        &self,
        identity: &shaula_core::github::ScaleSetIdentity,
        runner_group_id: i64,
        labels: &[Label],
    ) -> Result<EffectOutcome<ScaleSetView>, shaula_core::ports::AccessFailure> {
        let body = serde_json::json!({
            "name": identity.scale_set_name,
            "runnerGroupId": runner_group_id,
            "labels": labels.iter().map(|l| serde_json::json!({"name": l.name, "type": l.label_type})).collect::<Vec<_>>(),
            "RunnerSetting": {"disableUpdate": true},
        });
        let response = self
            .actions_service_request(
                reqwest::Method::POST,
                wire::SCALE_SET_ENDPOINT,
                &[],
                Some(body),
                crate::client::DEFAULT_REQUEST_TIMEOUT,
            )
            .await;
        let outcome = definite_or_uncertain::<wire::RunnerScaleSet>(response)
            .await
            .map_err(|e| e.to_access_failure())?;
        Ok(outcome.map(|view_wire| scale_set_view(&view_wire)))
    }

    async fn establish_session(
        &self,
        scale_set_id: i64,
        owner: &str,
    ) -> Result<EffectOutcome<SessionHandle>, shaula_core::ports::AccessFailure> {
        let path = format!("{}/{scale_set_id}/sessions", wire::SCALE_SET_ENDPOINT);
        let body = serde_json::json!({ "ownerName": owner });
        let response = self
            .actions_service_request(
                reqwest::Method::POST,
                &path,
                &[],
                Some(body),
                crate::client::DEFAULT_REQUEST_TIMEOUT,
            )
            .await;
        // R10-11: the session fields the listener DEPENDS on are required —
        // a response missing any of them is a malformed response, never a
        // silently defaulting empty id/url/token.
        let outcome = definite_or_uncertain::<wire::RunnerScaleSetSession>(response)
            .await
            .map_err(|e| e.to_access_failure())?;
        Ok(match outcome {
            EffectOutcome::Definite(session) => {
                let session_id = session
                    .session_id
                    .ok_or_else(|| shaula_core::ports::AccessFailure::Unavailable {
                        summary: "session response missing sessionId".into(),
                    })?
                    .to_string();
                let message_queue_url = session.message_queue_url.ok_or_else(|| {
                    shaula_core::ports::AccessFailure::Unavailable {
                        summary: "session response missing messageQueueUrl".into(),
                    }
                })?;
                let message_queue_access_token =
                    session.message_queue_access_token.ok_or_else(|| {
                        shaula_core::ports::AccessFailure::Unavailable {
                            summary: "session response missing messageQueueAccessToken".into(),
                        }
                    })?;
                self.service_url(&message_queue_url, "")
                    .map_err(|e| e.to_access_failure())?;
                if message_queue_access_token.is_empty() {
                    return Err(shaula_core::ports::AccessFailure::SessionExpired);
                }
                EffectOutcome::Definite(SessionHandle {
                    session_id,
                    message_queue_url,
                    message_queue_access_token,
                    initial_statistics: session.statistics.map(stats_from_wire).unwrap_or_default(),
                })
            }
            EffectOutcome::Uncertain { summary } => EffectOutcome::Uncertain { summary },
        })
    }

    async fn delete_session(
        &self,
        scale_set_id: i64,
        session_id: &str,
    ) -> Result<(), shaula_core::ports::AccessFailure> {
        self.delete_session_impl(scale_set_id, session_id).await
    }

    async fn acquire_jobs(
        &self,
        scale_set_id: i64,
        session: &shaula_core::ports::SessionHandle,
        request_ids: &[i64],
    ) -> Result<shaula_core::ports::EffectOutcome<Vec<i64>>, shaula_core::ports::AccessFailure>
    {
        self.acquire_jobs_impl(scale_set_id, session, request_ids)
            .await
    }

    async fn generate_jit(
        &self,
        scale_set_id: i64,
        runner_name: &str,
    ) -> Result<
        shaula_core::ports::EffectOutcome<shaula_core::ports::JitConfig>,
        shaula_core::ports::AccessFailure,
    > {
        self.generate_jit_impl(scale_set_id, runner_name).await
    }

    async fn get_runner_by_name(
        &self,
        scale_set_id: i64,
        runner_name: &str,
    ) -> Result<shaula_core::ports::RunnerLookup, shaula_core::ports::AccessFailure> {
        self.get_runner_by_name_impl(scale_set_id, runner_name)
            .await
    }

    async fn remove_runner(
        &self,
        runner_id: i64,
    ) -> Result<shaula_core::ports::RemovalOutcome, shaula_core::ports::AccessFailure> {
        self.remove_runner_impl(runner_id).await
    }

    async fn list_runners(
        &self,
        scale_set_id: i64,
    ) -> Result<Vec<shaula_core::ports::RunnerRef>, shaula_core::ports::AccessFailure> {
        self.list_runners_impl(scale_set_id).await
    }

    async fn poll_messages(
        &self,
        session: &SessionHandle,
        last_message_id: i64,
        max_capacity: i64,
    ) -> Result<PollOutcome, shaula_core::ports::AccessFailure> {
        self.poll_messages_impl(session, last_message_id, max_capacity)
            .await
    }

    async fn ack_message(
        &self,
        session: &SessionHandle,
        message_id: i64,
    ) -> Result<(), shaula_core::ports::AccessFailure> {
        self.ack_message_impl(session, message_id).await
    }

    fn allows_target(&self, target: &shaula_core::github::GitHubTarget) -> bool {
        self.config.target == *target
    }
}

#[path = "port_jobs.rs"]
mod port_jobs;

pub use port_jobs::parse_job_messages;

use port_jobs::{scale_set_view, stats_from_wire};

#[path = "port_ops.rs"]
mod port_ops;
