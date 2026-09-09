//! Read-only discovery of the verified GitHub App's installation page.

use shaula_core::secret::SecretString;

use crate::installation::AppInstallationResolver;
use crate::ScalesetError;

impl AppInstallationResolver {
    /// Authenticates the declared App with one `GET /app` and constructs its
    /// installation page on github.com. Remote URLs are never trusted, and no
    /// installation token or GitHub mutation is needed.
    ///
    /// # Errors
    /// Returns a bounded API error, an identity mismatch, or malformed metadata
    /// if GitHub does not return the declared numeric App id and a safe slug.
    pub async fn installation_url(
        &self,
        app_id: &str,
        private_key: &SecretString,
    ) -> Result<String, ScalesetError> {
        let response = self.app_request(app_id, private_key, "/app").await??;
        let body: serde_json::Value =
            response
                .json()
                .await
                .map_err(|_| ScalesetError::MalformedResponse {
                    summary: "app installation metadata invalid".into(),
                })?;
        installation_url_for_app(app_id, &body)
    }
}

fn installation_url_for_app(
    app_id: &str,
    body: &serde_json::Value,
) -> Result<String, ScalesetError> {
    let id = body
        .get("id")
        .and_then(serde_json::Value::as_i64)
        .filter(|id| crate::route_identity::valid_id(*id))
        .ok_or_else(|| ScalesetError::MalformedResponse {
            summary: "app identity response missing numeric id".into(),
        })?;
    if id.to_string() != app_id {
        return Err(ScalesetError::Configuration {
            summary: "authenticated app differs from the declared app id".into(),
        });
    }
    let slug = body
        .get("slug")
        .and_then(serde_json::Value::as_str)
        .filter(|slug| valid_slug(slug))
        .ok_or_else(|| ScalesetError::MalformedResponse {
            summary: "app installation metadata missing valid slug".into(),
        })?;
    Ok(format!("https://github.com/apps/{slug}/installations/new"))
}

fn valid_slug(slug: &str) -> bool {
    let alphanumeric = |byte: u8| byte.is_ascii_lowercase() || byte.is_ascii_digit();
    (1..=100).contains(&slug.len())
        && slug.bytes().next().is_some_and(alphanumeric)
        && slug.bytes().all(|byte| alphanumeric(byte) || byte == b'-')
}

#[cfg(test)]
mod tests;
