//! Inherent implementations backing the trait methods, split to keep
//! files under 400 lines (AGENTS.md).

use shaula_core::ports::{
    EffectOutcome, JitConfig, PollOutcome, RemovalOutcome, RunnerLookup, RunnerRef, SessionHandle,
};

use super::{
    definite_or_uncertain, port_jobs::parse_job_messages, ScalesetClient, LONG_POLL_TIMEOUT,
};
use crate::client::USER_AGENT;
use crate::error::{ActionsException, ScalesetError};
use crate::wire;

impl ScalesetClient {
    pub(crate) async fn delete_session_impl(
        &self,
        scale_set_id: i64,
        session_id: &str,
    ) -> Result<(), shaula_core::ports::AccessFailure> {
        let path = format!(
            "{}/{scale_set_id}/sessions/{session_id}",
            wire::SCALE_SET_ENDPOINT
        );
        let response = self
            .actions_service_request(
                reqwest::Method::DELETE,
                &path,
                &[],
                None,
                crate::client::DEFAULT_REQUEST_TIMEOUT,
            )
            .await;
        Self::expect_ok(response)
            .await
            .map(|_| ())
            .map_err(|e| e.to_access_failure())
    }

    pub(crate) async fn poll_messages_impl(
        &self,
        session: &SessionHandle,
        last_message_id: i64,
        max_capacity: i64,
    ) -> Result<PollOutcome, shaula_core::ports::AccessFailure> {
        let mut url = self
            .service_url(&session.message_queue_url, "")
            .map_err(|e| e.to_access_failure())?;
        if last_message_id > 0 {
            url.query_pairs_mut()
                .append_pair("lastMessageId", &last_message_id.to_string());
        }
        let response = self
            .http
            .get(url)
            .header(
                "Authorization",
                format!("Bearer {}", session.message_queue_access_token),
            )
            .header("User-Agent", USER_AGENT)
            .header("X-ScaleSetMaxCapacity", max_capacity.to_string())
            .timeout(LONG_POLL_TIMEOUT)
            .send()
            .await
            .map_err(crate::auth::transport_error_pub)
            .map_err(|e| e.to_access_failure())?;
        self.observe_response(&response);
        match response.status().as_u16() {
            202 => Ok(PollOutcome::NoMessage),
            200 => {
                let parsed = response
                    .json::<wire::RunnerScaleSetMessageResponse>()
                    .await
                    .map_err(|e| ScalesetError::MalformedResponse {
                        summary: format!("message body invalid: {e}"),
                    })
                    .map_err(|e| e.to_access_failure())?;
                if parsed.message_type != wire::MESSAGE_TYPE_JOB_MESSAGES {
                    return Err(shaula_core::ports::AccessFailure::Unavailable {
                        summary: "unsupported message type".into(),
                    });
                }
                let message = parse_job_messages(parsed)?;
                Ok(PollOutcome::Message(message))
            }
            401 => Ok(PollOutcome::SessionExpired),
            _ => Err(Self::parse_error(response).await.to_access_failure()),
        }
    }

    pub(crate) async fn ack_message_impl(
        &self,
        session: &SessionHandle,
        message_id: i64,
    ) -> Result<(), shaula_core::ports::AccessFailure> {
        let url = self
            .service_url(&session.message_queue_url, &message_id.to_string())
            .map_err(|e| e.to_access_failure())?;
        let response = self
            .http
            .delete(url)
            .header(
                "Authorization",
                format!("Bearer {}", session.message_queue_access_token),
            )
            .header("Content-Type", "application/json")
            .timeout(crate::client::DEFAULT_REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(crate::auth::transport_error_pub)
            .map_err(|e| e.to_access_failure())?;
        self.observe_response(&response);
        match response.status().as_u16() {
            204 => Ok(()),
            // Queue-token expiry: the caller re-establishes the session and
            // retries once (Go SDK parity), never falling back to credentials.
            401 => Err(shaula_core::ports::AccessFailure::SessionExpired),
            _ => Err(Self::parse_error(response).await.to_access_failure()),
        }
    }

    pub(crate) async fn acquire_jobs_impl(
        &self,
        scale_set_id: i64,
        session: &SessionHandle,
        request_ids: &[i64],
    ) -> Result<EffectOutcome<Vec<i64>>, shaula_core::ports::AccessFailure> {
        let path = format!("{}/{scale_set_id}/acquirejobs", wire::SCALE_SET_ENDPOINT);
        let conn = self
            .admin
            .connection()
            .await
            .inspect_err(|e| self.invalidate_route_proof(e))
            .map_err(|e| e.to_access_failure())?;
        let mut url = self
            .service_url(&conn.actions_service_url, &path)
            .map_err(|e| e.to_access_failure())?;
        url.query_pairs_mut()
            .append_pair("api-version", wire::API_VERSION);
        self.ensure_route_proof_impl().await?;
        let response = self
            .http
            .post(url)
            .header(
                "Authorization",
                format!("Bearer {}", session.message_queue_access_token),
            )
            .header("User-Agent", USER_AGENT)
            .json(&request_ids)
            .timeout(crate::client::DEFAULT_REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(crate::auth::transport_error_pub)
            .map_err(|e| e.to_access_failure())?;
        self.observe_response(&response);
        // Queue-token expiry: surface as SessionExpired so the caller
        // re-establishes the session and retries once (Go SDK parity).
        if response.status().as_u16() == 401 {
            return Err(shaula_core::ports::AccessFailure::SessionExpired);
        }
        let outcome = definite_or_uncertain::<wire::AcquireJobsResponse>(Ok(response))
            .await
            .map_err(|e| e.to_access_failure())?;
        Ok(outcome.map(|acquired| acquired.value))
    }

    pub(crate) async fn generate_jit_impl(
        &self,
        scale_set_id: i64,
        runner_name: &str,
    ) -> Result<EffectOutcome<JitConfig>, shaula_core::ports::AccessFailure> {
        let path = format!(
            "{}/{scale_set_id}/generatejitconfig",
            wire::SCALE_SET_ENDPOINT
        );
        let body = serde_json::json!({ "name": runner_name, "workFolder": "/_work" });
        let response = self.new_effect_request(&path, body).await;
        let outcome = definite_or_uncertain::<wire::RunnerScaleSetJitRunnerConfig>(response)
            .await
            .map_err(|e| e.to_access_failure())?;
        Ok(outcome.map(|config| {
            let runner_ref = config.runner.unwrap_or(wire::RunnerReference {
                id: 0,
                name: runner_name.to_string(),
                runner_scale_set_id: scale_set_id,
                status: String::new(),
            });
            JitConfig {
                encoded: config.encoded_jit_config,
                runner: RunnerRef {
                    id: runner_ref.id,
                    name: runner_ref.name,
                    scale_set_id,
                    // The mint response carries no status; the runner is
                    // by definition not online yet (spec 0024 grace).
                    status: "offline".to_string(),
                },
            }
        }))
    }

    pub(crate) async fn get_runner_by_name_impl(
        &self,
        scale_set_id: i64,
        runner_name: &str,
    ) -> Result<RunnerLookup, shaula_core::ports::AccessFailure> {
        if scale_set_id <= 0 {
            return Err(shaula_core::ports::AccessFailure::Unavailable {
                summary: "invalid scale set identity".into(),
            });
        }
        let response = self
            .actions_service_request(
                reqwest::Method::GET,
                wire::RUNNER_ENDPOINT,
                &[("agentName", runner_name.to_string())],
                None,
                crate::client::DEFAULT_REQUEST_TIMEOUT,
            )
            .await;
        let response = Self::expect_ok(response)
            .await
            .map_err(|e| e.to_access_failure())?;
        let list = response
            .json::<wire::RunnerReferenceList>()
            .await
            .map_err(|e| ScalesetError::MalformedResponse {
                summary: format!("runner list body invalid: {e}"),
            })
            .map_err(|e| e.to_access_failure())?;
        validate_runner_list(&list)?;
        match list.value.as_slice() {
            [] => Ok(RunnerLookup::None),
            [runner] => {
                if runner.name != runner_name || runner.runner_scale_set_id == 0 {
                    // A name match without membership is not ownership or
                    // authoritative absence of our runner.
                    return Err(shaula_core::ports::AccessFailure::Unavailable {
                        summary: "runner identity is not proven".into(),
                    });
                }
                if runner.runner_scale_set_id != scale_set_id {
                    // Same name in a different scale set: exact-name lookup
                    // for our scale set sees nothing.
                    Ok(RunnerLookup::None)
                } else {
                    Ok(RunnerLookup::ExactlyOne(RunnerRef {
                        id: runner.id,
                        name: runner.name.clone(),
                        scale_set_id: runner.runner_scale_set_id,
                        status: runner.status.clone(),
                    }))
                }
            }
            _ => Ok(RunnerLookup::Multiple),
        }
    }

    pub(crate) async fn remove_runner_impl(
        &self,
        runner_id: i64,
    ) -> Result<RemovalOutcome, shaula_core::ports::AccessFailure> {
        // Cleanup uses its retained revision/context, and must still prove
        // the target identity before deleting a runner through that route.
        let path = format!("{}/{runner_id}", wire::RUNNER_ENDPOINT);
        let response = match self
            .authorized_effect_request(reqwest::Method::DELETE, &path, None)
            .await
        {
            Ok(response) => response,
            Err(e) => return Err(e.to_access_failure()),
        };
        match response.status().as_u16() {
            204 => Ok(RemovalOutcome::Removed),
            // A raw 404 cannot be distinguished from an access-filtered
            // absence; treating it as AlreadyAbsent would authorize a
            // Destroy without the required readable-Scale-Set proof
            // (spec 0003 section 8). Fail closed instead.
            404 => Err(shaula_core::ports::AccessFailure::TargetHiddenOrNotFound),
            status if status == 409 || status == 400 => {
                let error = Self::parse_error(response).await;
                if let ScalesetError::Service {
                    exception: ActionsException::JobStillRunning,
                    ..
                } = &error
                {
                    Ok(RemovalOutcome::JobStillRunning)
                } else {
                    Err(error.to_access_failure())
                }
            }
            _ => {
                let error = Self::parse_error(response).await;
                Err(error.to_access_failure())
            }
        }
    }

    pub(crate) async fn list_runners_impl(
        &self,
        scale_set_id: i64,
    ) -> Result<Vec<RunnerRef>, shaula_core::ports::AccessFailure> {
        if scale_set_id <= 0 {
            return Err(shaula_core::ports::AccessFailure::Unavailable {
                summary: "invalid scale set identity".into(),
            });
        }
        let response = self
            .actions_service_request(
                reqwest::Method::GET,
                wire::RUNNER_ENDPOINT,
                &[],
                None,
                crate::client::DEFAULT_REQUEST_TIMEOUT,
            )
            .await;
        let response = Self::expect_ok(response)
            .await
            .map_err(|e| e.to_access_failure())?;
        let list = response
            .json::<wire::RunnerReferenceList>()
            .await
            .map_err(|e| ScalesetError::MalformedResponse {
                summary: format!("runner list body invalid: {e}"),
            })
            .map_err(|e| e.to_access_failure())?;
        validate_runner_list(&list)?;
        Ok(list
            .value
            .into_iter()
            .filter(|r| r.runner_scale_set_id == scale_set_id)
            .map(|r| RunnerRef {
                id: r.id,
                name: r.name,
                scale_set_id: r.runner_scale_set_id,
                status: r.status,
            })
            .collect())
    }
}

fn validate_runner_list(
    list: &wire::RunnerReferenceList,
) -> Result<(), shaula_core::ports::AccessFailure> {
    if usize::try_from(list.count).ok() != Some(list.value.len())
        || list
            .value
            .iter()
            // Target-wide inventories also contain ordinary runners. The
            // wire's omitted/default 0 membership is valid, but never proves
            // membership of a requested positive Scale Set.
            .any(|r| r.id <= 0 || r.name.is_empty() || r.runner_scale_set_id < 0)
    {
        return Err(shaula_core::ports::AccessFailure::Unavailable {
            summary: "inconsistent runner inventory".into(),
        });
    }
    Ok(())
}

#[cfg(test)]
#[path = "runner_inventory_tests.rs"]
mod tests;
