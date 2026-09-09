//! The optional delivery TTL starts after slow preparation and JIT complete.
use shaula_core::{ports::Clock, setup_info::SetupInfoIssuer, template::SetupInfoDescriptor};

pub(super) async fn issue(
    issuer: Option<&dyn SetupInfoIssuer>,
    clock: Option<&dyn Clock>,
    generation: &str,
) -> SetupInfoDescriptor {
    let (Some(issuer), Some(clock)) = (issuer, clock) else {
        return SetupInfoDescriptor::Disabled;
    };
    // Read the clock at issuance, never reuse the pre-init reconcile timestamp.
    issuer
        .issue(generation, clock.now_unix_ms().div_euclid(1000))
        .await
        .unwrap_or(SetupInfoDescriptor::Disabled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI64, Ordering};

    struct TestClock(AtomicI64);

    impl Clock for TestClock {
        fn now_unix_ms(&self) -> i64 {
            self.0.load(Ordering::Relaxed)
        }
    }

    struct Issuer;

    #[async_trait::async_trait]
    impl SetupInfoIssuer for Issuer {
        async fn issue(&self, _: &str, now: i64) -> shaula_core::CoreResult<SetupInfoDescriptor> {
            SetupInfoDescriptor::enabled(
                "https://logs.test/runner/v1/generations/test/setup-info".into(),
                "a".repeat(64),
                now + 60,
                60,
            )
        }
    }

    #[tokio::test]
    async fn slow_preparation_does_not_spend_the_capability_ttl() {
        let clock = TestClock(AtomicI64::new(1_700_000_000_000));
        let reconcile_started = clock.now_unix_ms();
        // Preparation and JIT take longer than the configured 60-second TTL.
        clock.0.fetch_add(125_999, Ordering::Relaxed);
        let descriptor = issue(Some(&Issuer), Some(&clock), "test").await;
        let expected = (reconcile_started + 125_999).div_euclid(1000) + 60;
        assert!(
            matches!(descriptor, SetupInfoDescriptor::Enabled { expires_at, .. } if expires_at == expected)
        );
    }

    #[tokio::test]
    async fn missing_clock_disables_delivery_instead_of_using_stale_time() {
        assert_eq!(
            issue(Some(&Issuer), None, "test").await,
            SetupInfoDescriptor::Disabled
        );
    }
}
