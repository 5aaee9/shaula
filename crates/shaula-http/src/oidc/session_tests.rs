#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
fn identity() -> Authenticated {
    Authenticated {
        actor: shaula_core::registry::Actor {
            name: "principal".into(),
            scopes: vec![],
        },
        name: "Display".into(),
        csrf: None,
    }
}
fn transaction() -> Transaction {
    Transaction {
        nonce: random(),
        binding: "browser".into(),
        verifier: PkceCodeVerifier::new(random()),
        target: "/fleets".into(),
        expires: Instant::now() + Duration::from_secs(600),
        scopes: vec!["openid".into()],
    }
}

fn refresh_grant() -> RefreshGrant {
    RefreshGrant {
        token: SecretString::new("test-refresh-token"),
        binding: BrowserBinding {
            subject: "ops".into(),
            audience: crate::oidc::tokens::Audience::One("web".into()),
            nonce: "login-nonce".into(),
            auth_time: None,
        },
        scopes: vec!["openid".into()],
    }
}

#[test]
fn oidc_tests_expiry_idle_fixation_and_restart() {
    let mut store = Sessions::default();
    let exp = jsonwebtoken::get_current_timestamp() + 7200;
    let (old, seconds) = store.create(identity(), exp, None).unwrap();
    assert_eq!(seconds, 3600);
    let old_csrf = store.get(&old).unwrap().csrf;
    let (new, _) = store.create(identity(), exp, Some(&old)).unwrap();
    assert_ne!(old, new);
    assert!(store.get(&old).is_err());
    assert_ne!(old_csrf, store.get(&new).unwrap().csrf);
    store.sessions.get_mut(&new).unwrap().idle = Instant::now();
    assert!(store.get(&new).is_err());
    let (id, _) = store.create(identity(), exp, None).unwrap();
    store.sessions.get_mut(&id).unwrap().expires = Instant::now();
    assert!(store.get(&id).is_err());
    let (id, _) = store.create(identity(), exp, None).unwrap();
    assert!(Sessions::default().get(&id).is_err());
    assert!(store.create(identity(), 0, None).is_err());
}

#[test]
fn oidc_tests_store_limits_evict_and_transactions_are_one_use() {
    let mut store = Sessions::default();
    for i in 0..LIMIT {
        store.begin(i.to_string(), transaction()).unwrap();
    }
    assert!(matches!(
        store.begin("full".into(), transaction()),
        Err(AuthError::Capacity)
    ));
    assert!(store.consume("0", "wrong-browser").is_err());
    assert!(store.consume("0", "browser").is_err());
    store.transactions.get_mut("1").unwrap().expires = Instant::now();
    assert!(store.consume("1", "browser").is_err());
    store.begin("new".into(), transaction()).unwrap();
    store.consume("new", "browser").unwrap();
    assert!(store.consume("new", "browser").is_err());
    for i in 0..LIMIT {
        store.sessions.insert(
            i.to_string(),
            Session {
                identity: identity(),
                expires: Instant::now() + Duration::from_secs(60),
                idle: Instant::now() + Duration::from_secs(60),
                refresh: None,
                renewal: RenewalState::Idle,
            },
        );
    }
    assert!(matches!(
        store.create(identity(), jsonwebtoken::get_current_timestamp() + 60, None),
        Err(AuthError::Capacity)
    ));
    store.sessions.get_mut("0").unwrap().expires = Instant::now();
    assert!(store
        .create(identity(), jsonwebtoken::get_current_timestamp() + 60, None)
        .is_ok());
}

#[test]
fn oidc_tests_cookie_ambiguity_and_return_targets() {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::COOKIE,
        "__Host-shaula-session=one; __Host-shaula-session=two"
            .parse()
            .unwrap(),
    );
    assert!(cookie_value(&headers, SESSION).is_err());
    for target in [
        "//evil.example",
        "https://evil.example",
        "/fleets\\evil",
        "/%2f%2fevil",
        "/fleets?key=%252f%252fevil",
        "/unknown",
        "/auth?return_to=evil",
    ] {
        assert_eq!(crate::oidc::login::return_target(Some(target)), "/fleets");
    }
    assert_eq!(
        crate::oidc::login::return_target(Some("/changes?type=fleet&id=job-1")),
        "/changes?type=fleet&id=job-1"
    );
}

#[test]
fn oidc_tests_template_publish_return_targets_require_valid_routes() {
    for target in [
        "/templates/new",
        "/templates/linux-build/revisions/new",
        "/templates/linux-x64.1_a/revisions/new",
        "/templates/linux-build/update",
        "/templates/linux-x64.1_a/update",
        "/templates?key=linux-build",
    ] {
        assert_eq!(crate::oidc::login::return_target(Some(target)), target);
    }
    for target in [
        "/templates/linux-build",
        "/templates/new/extra",
        "/templates//revisions/new",
        "/templates/./revisions/new",
        "/templates/../revisions/new",
        "/templates/%2e%2e/revisions/new",
        "/templates/with%20space/revisions/new",
        "/templates/a/b/revisions/new",
        "/templates/linux-build/revisions/latest",
        "/templates/linux-build/revisions/new/",
        "/templates//update",
        "/templates/../update",
        "/templates/%2e%2e/update",
        "/templates/a/b/update",
        "/templates/linux-build/update/",
        "/templates/linux-build/update?return_to=https://evil.example",
        "/templates/new?return_to=https://evil.example",
        "/templates/new#https://evil.example",
    ] {
        assert_eq!(crate::oidc::login::return_target(Some(target)), "/fleets");
    }
    let oversized_key = format!("/templates/{}/revisions/new", "a".repeat(129));
    assert_eq!(
        crate::oidc::login::return_target(Some(&oversized_key)),
        "/fleets"
    );
}

#[test]
fn oidc_tests_jobs_return_targets_preserve_filters_only_on_valid_documents() {
    for target in [
        "/jobs",
        "/jobs/job-1",
        "/jobs/runners/gen-1",
        "/jobs?fleet_key=linux&repository=acme%2Frepo&job_name=Build+Linux",
        "/jobs/runners?fleet_key=linux&cursor=abc123",
    ] {
        assert_eq!(crate::oidc::login::return_target(Some(target)), target);
    }
    for target in [
        "/jobs//evil",
        "/jobs/%2e%2e",
        "/jobs/job-1/extra",
        "/jobs?return_to=https://evil.example",
        "/jobs?job_name=%0D%0ALocation:evil",
        "/jobs?status=running&status=completed",
    ] {
        assert_eq!(crate::oidc::login::return_target(Some(target)), "/fleets");
    }
}

#[test]
fn oidc_tests_refreshable_idle_expiry_requires_renewal_without_local_logout() {
    use crate::oidc::session_refresh::Admission;
    let mut store = Sessions::default();
    let id = store
        .insert(
            identity(),
            Instant::now() + Duration::from_secs(60),
            Some(refresh_grant()),
            None,
        )
        .unwrap();
    let csrf = store.get(&id).unwrap().csrf;
    store.sessions.get_mut(&id).unwrap().idle = Instant::now();
    assert!(
        store.get(&id).is_err(),
        "idle identity is not fresh authority"
    );
    assert!(matches!(store.admission(&id), Ok(Admission::Renew(_))));
    assert_eq!(
        store.retained(&id).unwrap().csrf,
        csrf,
        "stale logout keeps its binding"
    );
    assert!(
        store.get(&id).is_err(),
        "retention/CSRF inspection does not extend idle"
    );
    let cookie = session_cookie(id);
    for attribute in ["Secure", "HttpOnly", "SameSite=Lax", "Path=/"] {
        assert!(cookie.contains(attribute));
    }
    assert!(!cookie.contains("Max-Age"));
    assert!(!cookie.contains("Expires"));
}

#[test]
fn oidc_tests_provider_owned_lifetime_keeps_expired_refreshable_slots_bounded() {
    use crate::oidc::session_refresh::Admission;
    let mut store = Sessions::default();
    let now = Instant::now();
    // Windows' monotonic clock may not represent a date before system startup.
    let ancient = now
        .checked_sub(Duration::from_secs(31 * 24 * 3600))
        .unwrap_or(now);
    for i in 0..LIMIT {
        store.sessions.insert(
            i.to_string(),
            Session {
                identity: identity(),
                expires: ancient,
                idle: ancient,
                refresh: Some(refresh_grant()),
                renewal: RenewalState::Idle,
            },
        );
    }
    assert!(
        matches!(store.admission("0"), Ok(Admission::Renew(_))),
        "only the Provider may decide that the retained token expired"
    );
    assert_eq!(store.sessions.len(), LIMIT);
    assert!(matches!(
        store.create(identity(), jsonwebtoken::get_current_timestamp() + 60, None),
        Err(AuthError::Capacity)
    ));
    store.remove("0");
    assert!(store
        .create(identity(), jsonwebtoken::get_current_timestamp() + 60, None)
        .is_ok());
    assert_eq!(store.sessions.len(), LIMIT);
    assert!(store.retained("0").is_err());
    assert!(
        Sessions::default().retained("1").is_err(),
        "restart has no retained credentials"
    );
}

#[test]
fn oidc_tests_refresh_success_preserves_csrf_and_removed_sessions_stay_removed() {
    use crate::oidc::session_refresh::{Flight, Refreshed};
    let mut store = Sessions::default();
    let id = store
        .insert(
            identity(),
            Instant::now() + Duration::from_secs(60),
            Some(refresh_grant()),
            None,
        )
        .unwrap();
    let csrf = store.get(&id).unwrap().csrf;
    let flight = Flight::new();
    store.start(&id, flight.clone()).unwrap();
    store
        .finish(
            &id,
            &flight,
            Ok(Refreshed {
                identity: identity(),
                expires: Instant::now() + Duration::from_secs(60),
                token: Some(SecretString::new("rotated-refresh-token")),
            }),
        )
        .unwrap();
    assert_eq!(store.get(&id).unwrap().csrf, csrf);
    assert_eq!(
        store.sessions[&id].refresh.as_ref().unwrap().token.expose(),
        "rotated-refresh-token"
    );
    store.start(&id, flight.clone()).unwrap();
    store.remove(&id);
    assert!(store
        .finish(
            &id,
            &flight,
            Ok(Refreshed {
                identity: identity(),
                expires: Instant::now() + Duration::from_secs(60),
                token: None,
            })
        )
        .is_err());
    assert!(store.retained(&id).is_err());
}
