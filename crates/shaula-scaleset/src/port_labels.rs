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
        let response = self
            .authorized_effect_request(
                reqwest::Method::PATCH,
                &path,
                Some(serde_json::json!({"labels": labels})),
            )
            .await;
        let outcome = definite_or_uncertain::<wire::RunnerScaleSet>(response)
            .await
            .map_err(|error| error.to_access_failure())?;
        Ok(outcome.map(|view| scale_set_view(&view)))
    }
}
