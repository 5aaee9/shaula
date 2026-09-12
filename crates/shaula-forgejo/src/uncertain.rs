use crate::client::normalize_label_names;
use crate::error::ForgejoError;
use crate::models::{ForgejoScope, RegistrationUncertainty, Runner};
use crate::ForgejoClient;

impl ForgejoClient {
    pub async fn classify_uncertain_registration(
        &self,
        name: &str,
        labels: &[String],
    ) -> Result<RegistrationUncertainty, ForgejoError> {
        if name.trim().is_empty() {
            return Err(ForgejoError::Configuration(
                "runner name must not be empty".into(),
            ));
        }
        let labels = normalize_label_names(labels)?;
        let matches = self
            .list_runners()
            .await?
            .into_iter()
            .filter(|runner| runner.name == name && self.runner_belongs_to_scope(runner))
            .collect::<Vec<_>>();
        Ok(classify_matches(&matches, &labels))
    }

    /// Returns only candidates for which the API provides the joint
    /// ownership evidence required by the spec. In particular, an
    /// immediately-after-registration runner has no declared labels yet and
    /// is therefore not a cleanup candidate: guessing from its name would
    /// risk deleting an unrelated runner.
    pub async fn uncertain_registration_candidates(
        &self,
        name: &str,
        labels: &[String],
    ) -> Result<Vec<Runner>, ForgejoError> {
        if name.trim().is_empty() {
            return Err(ForgejoError::Configuration(
                "runner name must not be empty".into(),
            ));
        }
        let labels = normalize_label_names(labels)?;
        let runners = self.list_runners().await?;
        Ok(runners
            .into_iter()
            .filter(|runner| {
                runner.name == name
                    && runner.ephemeral
                    && !runner.labels.is_empty()
                    && runner.has_labels(&labels)
                    && self.runner_belongs_to_scope(runner)
            })
            .collect())
    }

    fn runner_belongs_to_scope(&self, runner: &Runner) -> bool {
        match self.scope() {
            ForgejoScope::Repository { .. } => runner.repo_id != 0,
            ForgejoScope::Instance | ForgejoScope::Organization(_) | ForgejoScope::User => true,
        }
    }
}

fn classify_matches(matches: &[Runner], labels: &[String]) -> RegistrationUncertainty {
    match matches {
        [] => RegistrationUncertainty::None,
        [runner]
            if runner.ephemeral
                && !runner.labels.is_empty()
                && runner.has_labels(labels)
                && runner.is_known()
                && !runner.is_active() =>
        {
            RegistrationUncertainty::ExactlyOneCleanupRequired
        }
        // A same-name runner with missing/mismatched label evidence is not
        // absence. Before Declare, response loss must quarantine it.
        _ => RegistrationUncertainty::Quarantined,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn runner(status: &str, labels: Vec<&str>) -> Result<Runner, serde_json::Error> {
        serde_json::from_value(serde_json::json!({
            "id": 42,
            "uuid": "uuid-42",
            "name": "runner-42",
            "status": status,
            "labels": labels,
            "ephemeral": true
        }))
    }

    #[test]
    fn undeclared_or_busy_same_name_is_quarantined_not_absent() -> TestResult {
        let labels = vec!["linux".into()];
        for candidate in [runner("offline", vec![])?, runner("active", vec!["linux"])?] {
            assert_eq!(
                classify_matches(&[candidate], &labels),
                RegistrationUncertainty::Quarantined
            );
        }
        assert_eq!(
            classify_matches(&[], &labels),
            RegistrationUncertainty::None
        );
        assert_eq!(
            classify_matches(&[runner("idle", vec!["linux"])?], &labels),
            RegistrationUncertainty::ExactlyOneCleanupRequired
        );
        Ok(())
    }
}
