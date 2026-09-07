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
