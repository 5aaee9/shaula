//! Label names select jobs; full labels also select the runner execution backend.

use crate::error::ForgejoError;

pub(crate) fn normalize_runner_labels(labels: &[String]) -> Result<Vec<String>, ForgejoError> {
    // An empty filter is meaningful to the low-level API, but is not admitted
    // as a Fleet configuration (core enforces a nonempty set).
    if labels.is_empty() {
        return Ok(Vec::new());
    }
    shaula_core::forgejo::validate_labels(labels)
        .map_err(|error| ForgejoError::Configuration(error.summary))?;
    Ok(labels.to_vec())
}

pub(crate) fn normalize_label_names(labels: &[String]) -> Result<Vec<String>, ForgejoError> {
    Ok(normalize_runner_labels(labels)?
        .into_iter()
        .map(|label| label.split(':').next().unwrap_or_default().to_string())
        .collect())
}
