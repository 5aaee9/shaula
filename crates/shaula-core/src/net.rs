//! Network policy primitives owned by the core.

use std::net::IpAddr;

/// Validate that a bind address is loopback. Non-loopback binds fail
/// closed at startup (ADR-0011); the production binary never exposes a
/// non-loopback listener.
pub fn verify_loopback(host: &str) -> Result<(), String> {
    let ip: IpAddr = host
        .parse()
        .map_err(|_| format!("bind address {host:?} is not a valid IP"))?;
    if ip.is_loopback() {
        Ok(())
    } else {
        Err(format!(
            "non-loopback bind {host:?} rejected; expose remote access through a trusted reverse proxy"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_only() {
        assert!(verify_loopback("127.0.0.1").is_ok());
        assert!(verify_loopback("::1").is_ok());
        assert!(verify_loopback("0.0.0.0").is_err());
        assert!(verify_loopback("10.0.0.1").is_err());
    }
}
