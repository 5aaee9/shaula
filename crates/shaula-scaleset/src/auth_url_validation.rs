//! Credential destinations are parsed before constructing requests.

use crate::error::ScalesetError;

/// Validates the Actions Service URL returned by the registration
/// response — the bearer-bearing requests go exactly there, so a hostile
/// value (wrong scheme, userinfo, missing host) must be refused, never
/// silently accepted. Returns the cleaned URL (no trailing slash).
pub(crate) fn validate_actions_service_url(
    raw: &str,
    allow_test_endpoints: bool,
) -> Result<reqwest::Url, ScalesetError> {
    let invalid = || ScalesetError::MalformedResponse {
        summary: "untrusted Actions Service credential destination".into(),
    };
    if raw.chars().any(|c| c.is_whitespace() || c.is_control()) || raw.contains('\\') {
        return Err(invalid());
    }
    let url = reqwest::Url::parse(raw).map_err(|_| invalid())?;
    let host = url.host_str().ok_or_else(invalid)?;
    let trusted = url.scheme() == "https"
        && url.port_or_known_default() == Some(443)
        && (host == "actions.githubusercontent.com"
            || host.ends_with(".actions.githubusercontent.com"));
    let loopback = host == "localhost"
        || host
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
    let test = cfg!(debug_assertions)
        && allow_test_endpoints
        && loopback
        && matches!(url.scheme(), "http" | "https");
    if !(trusted || test)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.query().is_some()
    {
        return Err(invalid());
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_untrusted_and_ambiguous_destinations() {
        for url in [
            "https://queue.invalid/",
            "https://actions.githubusercontent.com.evil.test/",
            "https://user@a.actions.githubusercontent.com/",
            "https://a.actions.githubusercontent.com:444/",
            "https://a.actions.githubusercontent.com/#fragment",
            "https://a.actions.githubusercontent.com/?token=x",
            "http://127.0.0.1.evil.test/",
            "http://localhost.evil.test/",
            "http://127.0.0.1/",
            "https://a.actions.githubusercontent.com/\n",
        ] {
            assert!(validate_actions_service_url(url, false).is_err(), "{url}");
        }
        assert!(validate_actions_service_url(
            "https://a.actions.githubusercontent.com/service/",
            false
        )
        .is_ok());
        assert!(validate_actions_service_url("http://127.0.0.1:1234/", true).is_ok());
        assert!(validate_actions_service_url("http://[::1]:1234/", true).is_ok());
        assert!(validate_actions_service_url("http://localhost.evil.test/", true).is_err());
    }
}
