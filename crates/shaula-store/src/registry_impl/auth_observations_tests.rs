use super::*;
use shaula_core::{
    auth_context::ResolvedAuthContext, auth_policy::AccountKind, github::GitHubTarget,
};

fn observation(fleet: &str, healthy: bool, checked: i64) -> AuthRouteObservation {
    AuthRouteObservation {
        fleet_key: fleet.into(),
        checked_at_ms: checked,
        valid_until_ms: i64::MAX,
        healthy,
        reason: None,
        context: ResolvedAuthContext {
            profile_key: "app".into(),
            revision: 1,
            github_host: "github.com".into(),
            app_id: "123".into(),
            account_id: 1,
            account_kind: AccountKind::Organization,
            login: "org".into(),
            installation_id: 2,
            target: GitHubTarget::Organization {
                owner: "org".into(),
            },
            organization_id: Some(1),
            repository_id: None,
            repository_owner_id: None,
        },
    }
}

#[test]
fn observations_expire_are_ref_scoped_and_out_of_order_completions_cannot_overwrite() {
    let cache = RouteObservations::default();
    cache.report(observation("one", true, 100));
    cache.report(observation("one", false, 99));
    let values = cache.read("app", 1, 100);
    assert_eq!(values.len(), 1);
    assert!(values[0].healthy);
    assert_eq!(values[0].valid_until_ms, 60_100);
    assert!(cache.read("app", 2, 100).is_empty());
    assert!(cache.read("other", 1, 100).is_empty());
    assert!(cache.read("app", 1, 60_100).is_empty());
    cache.report(observation("one", false, 70_000));
    assert_eq!(cache.read("app", 1, 70_000)[0].valid_until_ms, 85_000);
    assert!(cache.read("app", 1, 85_000).is_empty());
    assert!(
        RouteObservations::default()
            .read("app", 1, 70_000)
            .is_empty(),
        "restart is Unknown"
    );
}

#[test]
fn bounded_cache_evicts_oldest_without_growing_or_relabeling_other_fleets() {
    let cache = RouteObservations::default();
    for index in 0..=MAX_OBSERVATIONS {
        cache.report(observation(&index.to_string(), true, index as i64));
    }
    let values = cache.read("app", 1, MAX_OBSERVATIONS as i64);
    assert_eq!(values.len(), MAX_OBSERVATIONS);
    assert!(!values.iter().any(|entry| entry.fleet_key == "0"));
    assert!(values
        .iter()
        .any(|entry| entry.fleet_key == MAX_OBSERVATIONS.to_string()));
}
