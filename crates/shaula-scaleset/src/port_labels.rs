//! Narrow routing update for a scale set whose ownership is already proven.

use shaula_core::github::Label;
use shaula_core::ports::{AccessFailure, EffectOutcome, ScaleSetView};

use super::{definite_or_uncertain, scale_set_view};
use crate::{wire, ScalesetClient};

impl ScalesetClient {
    pub(crate) async fn update_scale_set_labels_impl(
        &self,
        scale_set_id: i64,
        labels: &[Label],
    ) -> Result<EffectOutcome<ScaleSetView>, AccessFailure> {
        let path = format!("{}/{scale_set_id}", wire::SCALE_SET_ENDPOINT);
        // GitHub accepts PATCH but leaves labels unchanged. PUT replaces the
        // label set while omitted identity and runner settings stay intact.
        let response = self
            .authorized_effect_request(
                reqwest::Method::PUT,
                &path,
                Some(serde_json::json!({"labels": labels})),
            )
            .await;
        let http_status = response.as_ref().ok().map(|reply| reply.status().as_u16());
        let outcome = definite_or_uncertain::<wire::RunnerScaleSet>(response)
            .await
            .inspect_err(|_| {
                tracing::warn!(scale_set_id, http_status, "scale set label update failed");
            })
            .map_err(|error| error.to_access_failure())?;
        Ok(outcome.map(|view| scale_set_view(&view)))
    }
}
