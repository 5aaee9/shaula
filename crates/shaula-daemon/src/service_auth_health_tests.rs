#![allow(clippy::unwrap_used)]
use super::*;
use shaula_core::auth_context::RepositorySelection;
use shaula_core::auth_policy::AccountKind;
use shaula_core::github::GitHubTarget;

fn binding_health(
    key: &str,
    revision: i64,
    binding: &AccountBinding,
    routes: &[(FleetAuthContextRow, Option<AuthHandoffRow>)],
    observations: &[AuthRouteObservation],
    now: i64,
) -> AuthBindingHealth {
    super::binding_health(key, revision, binding, routes, observations, &[], now)
}

fn binding() -> AccountBinding {
    AccountBinding {
        account_id: 10,
        account_kind: AccountKind::User,
        login: "person".into(),
        installation_id: 20,
        repository_selection: RepositorySelection::All,
        validated_at_ms: 100,
    }
}

fn route(fleet: &str, revision: i64, state: &str) -> (FleetAuthContextRow, Option<AuthHandoffRow>) {
    let context = ResolvedAuthContext {
        profile_key: "app".into(),
        revision,
        github_host: "github.com".into(),
        app_id: "123".into(),
        account_id: 10,
        account_kind: AccountKind::User,
        login: "person".into(),
        installation_id: 20,
        target: GitHubTarget::new_repository("person", "repo").unwrap(),
        organization_id: None,
        repository_id: Some(30),
        repository_owner_id: Some(10),
    };
    (
        FleetAuthContextRow {
            fleet_key: fleet.into(),
            desired: Some(("app".into(), revision)),
            desired_context_json: Some(serde_json::to_string(&context).unwrap()),
            observed: None,
            observed_context_json: None,
            state: state.into(),
            reason: None,
        },
        None,
    )
}

#[test]
fn candidate_validation_expires_and_never_means_current_access_is_healthy() {
    let binding = binding();
    let fresh = binding_health("app", 1, &binding, &[], &[], 100);
    assert_eq!(fresh.state, "Validated");
    assert_eq!(fresh.reason.as_deref(), Some("CandidateValidationOnly"));
    assert_eq!(fresh.valid_until_ms, Some(60_100));
    let expired = binding_health("app", 1, &binding, &[], &[], 60_100);
    assert_eq!(expired.state, "Unknown");
    assert_eq!(expired.checked_at_ms, None);
    assert_eq!(
        binding_health("app", 1, &binding, &[], &[], 99).state,
        "Unknown"
    );
}

#[test]
fn candidate_or_other_account_failures_do_not_taint_active_binding() {
    let mut other_account = route("other-account", 1, "Blocked");
    let mut context: ResolvedAuthContext =
        serde_json::from_str(other_account.0.desired_context_json.as_deref().unwrap()).unwrap();
    context.account_id = 11;
    other_account.0.desired_context_json = Some(serde_json::to_string(&context).unwrap());
    let health = binding_health(
        "app",
        1,
        &binding(),
        &[route("candidate", 2, "Blocked"), other_account],
        &[],
        100,
    );
    assert_eq!(health.state, "Validated");
    assert!(health.affected_fleets.is_empty());
}

#[test]
fn exact_route_failures_report_blocked_or_degraded_and_recover_conservatively() {
    let blocked = route("blocked", 1, "Blocked");
    let healthy = route("observed", 1, "Observed");
    assert_eq!(
        binding_health(
            "app",
            1,
            &binding(),
            std::slice::from_ref(&blocked),
            &[],
            60_100
        )
        .state,
        "Blocked"
    );
    let mixed = binding_health(
        "app",
        1,
        &binding(),
        &[blocked, healthy.clone()],
        &[],
        60_100,
    );
    assert_eq!(mixed.state, "Degraded");
    assert_eq!(mixed.affected_fleets, ["blocked"]);
    assert_eq!(
        binding_health("app", 1, &binding(), &[healthy], &[], 60_100).state,
        "Unknown"
    );
}

#[test]
fn actual_access_observations_are_bounded_and_never_cross_revision_or_account() {
    let routes = vec![route("one", 1, "Observed"), route("two", 1, "Observed")];
    let context: ResolvedAuthContext =
        serde_json::from_str(routes[0].0.desired_context_json.as_deref().unwrap()).unwrap();
    let one = AuthRouteObservation {
        fleet_key: "one".into(),
        context,
        checked_at_ms: 70_000,
        valid_until_ms: 85_000,
        healthy: false,
        reason: Some("InstallationSuspended".into()),
    };
    let health = binding_health(
        "app",
        1,
        &binding(),
        &routes,
        std::slice::from_ref(&one),
        70_000,
    );
    assert_eq!(health.state, "Degraded");
    assert_eq!(health.reason.as_deref(), Some("InstallationSuspended"));
    assert_eq!(health.valid_until_ms, Some(85_000));
    assert_eq!(
        binding_health(
            "app",
            1,
            &binding(),
            &routes,
            std::slice::from_ref(&one),
            85_000
        )
        .state,
        "Unknown"
    );
    assert_eq!(
        binding_health(
            "app",
            2,
            &binding(),
            &routes,
            std::slice::from_ref(&one),
            70_000
        )
        .state,
        "Unknown"
    );
    let mut other = one.clone();
    other.context.account_id = 11;
    assert_eq!(
        binding_health("app", 1, &binding(), &routes, &[other], 70_000).state,
        "Unknown"
    );
    let mut healthy_one = one;
    healthy_one.healthy = true;
    let mut healthy_two = healthy_one.clone();
    healthy_two.fleet_key = "two".into();
    assert_eq!(
        binding_health(
            "app",
            1,
            &binding(),
            &routes,
            &[healthy_one, healthy_two],
            70_000
        )
        .state,
        "Healthy"
    );
}

#[test]
fn retained_observed_revision_uses_the_same_fleet_set_for_health_coverage() {
    let old = route("one", 1, "Observed");
    let mut rotating = route("one", 2, "Blocked");
    rotating.0.observed = old.0.desired.clone();
    rotating.0.observed_context_json = old.0.desired_context_json.clone();
    let context = serde_json::from_str(old.0.desired_context_json.as_deref().unwrap()).unwrap();
    let observation = AuthRouteObservation {
        fleet_key: "one".into(),
        context,
        checked_at_ms: 70_000,
        valid_until_ms: 130_000,
        healthy: true,
        reason: None,
    };
    assert_eq!(
        binding_health(
            "app",
            1,
            &binding(),
            std::slice::from_ref(&rotating),
            std::slice::from_ref(&observation),
            70_000
        )
        .state,
        "Healthy"
    );
    // A different Fleet still desires the old revision but has no current proof.
    // Success of only the retained observed route cannot imply whole-account health.
    let routes = [rotating, route("two", 1, "Pending")];
    assert_eq!(
        binding_health("app", 1, &binding(), &routes, &[observation], 70_000).state,
        "Unknown"
    );
}

#[test]
fn live_generation_context_remains_in_health_after_both_fleet_heads_advance() {
    let old = route("one", 1, "Observed");
    let context: ResolvedAuthContext =
        serde_json::from_str(old.0.desired_context_json.as_deref().unwrap()).unwrap();
    let observation = AuthRouteObservation {
        fleet_key: "one".into(),
        context: context.clone(),
        checked_at_ms: 70_000,
        valid_until_ms: 85_000,
        healthy: false,
        reason: Some("PermissionDenied".into()),
    };
    let current = route("one", 2, "Observed");
    let health = super::binding_health(
        "app",
        1,
        &binding(),
        &[current],
        &[observation],
        &[("one", &context)],
        70_000,
    );
    assert_eq!(health.state, "Blocked");
    assert_eq!(health.reason.as_deref(), Some("PermissionDenied"));
}
