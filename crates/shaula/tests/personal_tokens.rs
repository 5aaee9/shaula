use shaula_core::{
    access_tokens::*,
    ports::Clock,
    registry::{Actor, Scope},
};
use shaula_daemon::access_tokens::PersonalTokens;
use shaula_store::Store;
use std::sync::{
    atomic::{AtomicI64, Ordering},
    Arc,
};
type TestResult = Result<(), Box<dyn std::error::Error>>;
struct Time(AtomicI64);
impl Clock for Time {
    fn now_unix_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

async fn fixture(
    policy: TokenPolicy,
) -> Result<(tempfile::TempDir, Store, Arc<Time>, Actor, PersonalTokens), Box<dyn std::error::Error>>
{
    let dir = tempfile::tempdir()?;
    let store = Store::open(&dir.path().join("tokens.db")).await?;
    store.migrate().await?;
    let clock = Arc::new(Time(AtomicI64::new(1_800_000_000_000)));
    let actor = Actor {
        authentication: Default::default(),
        name: serde_json::to_string(&("oidc-v1", "https://id.test", "one"))?,
        scopes: Scope::ALL.to_vec(),
    };
    let grants = [(actor.name.clone(), actor.scopes.clone())].into();
    let service = PersonalTokens::new(
        Arc::new(store.clone()),
        clock.clone(),
        policy,
        "realm".into(),
        grants,
    )?;
    Ok((dir, store, clock, actor, service))
}
fn body() -> IssueToken {
    IssueToken {
        name: "CI".into(),
        scopes: vec!["fleet.read".into()],
        expires_in_seconds: Some(60),
    }
}

#[tokio::test]
async fn disabled_policy_keeps_metadata_and_revoke_and_lower_ttl_does_not_break_recovery(
) -> TestResult {
    let (_dir, store, clock, actor, service) = fixture(Default::default()).await?;
    let mut request = body();
    request.expires_in_seconds = Some(3600);
    let first = service
        .issue(&actor, true, request.clone(), "recover")
        .await?;
    let raw = first.secret.ok_or("secret")?;
    let constrained = PersonalTokens::new(
        Arc::new(store.clone()),
        clock.clone(),
        TokenPolicy {
            default_ttl_secs: 60,
            max_ttl_secs: 60,
            ..Default::default()
        },
        "realm".into(),
        [(actor.name.clone(), actor.scopes.clone())].into(),
    )?;
    assert!(constrained
        .issue(&actor, true, request.clone(), "recover")
        .await?
        .secret
        .is_none());
    assert!(matches!(
        constrained.issue(&actor, true, request, "new").await,
        Err(TokenError::Invalid)
    ));
    let disabled = PersonalTokens::new(
        Arc::new(store),
        clock,
        TokenPolicy {
            enabled: false,
            ..Default::default()
        },
        "realm".into(),
        [(actor.name.clone(), actor.scopes.clone())].into(),
    )?;
    assert!(matches!(
        disabled.authenticate(raw.expose()).await,
        Err(TokenError::Unauthorized)
    ));
    assert_eq!(
        disabled
            .get(&actor.name, &first.access_token.id)
            .await?
            .state,
        "disabled"
    );
    disabled
        .revoke(&actor, &first.access_token.id, Some(1))
        .await?;
    assert_eq!(
        disabled
            .get(&actor.name, &first.access_token.id)
            .await?
            .state,
        "revoked"
    );
    Ok(())
}

#[tokio::test]
async fn quota_race_and_rate_limit_never_commit_extra_token_rows() -> TestResult {
    let (_dir, _store, clock, actor, service) = fixture(TokenPolicy {
        max_active_per_principal: 1,
        ..Default::default()
    })
    .await?;
    let (a, b) = tokio::join!(
        service.issue(&actor, true, body(), "a"),
        service.issue(&actor, true, body(), "b")
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert!(matches!(a, Err(TokenError::Quota)) || matches!(b, Err(TokenError::Quota)));
    for n in 0..4 {
        let page = service.list(&actor.name, TokenQuery::default()).await?;
        for item in page.items {
            service.revoke(&actor, &item.id, None).await?;
        }
        service
            .issue(&actor, true, body(), &format!("rate-{n}"))
            .await?;
    }
    assert!(matches!(
        service.issue(&actor, true, body(), "limited").await,
        Err(TokenError::RateLimited)
    ));
    clock.0.fetch_add(60_000, Ordering::SeqCst);
    assert!(service
        .issue(&actor, true, body(), "next-window")
        .await
        .is_ok());
    Ok(())
}

#[tokio::test]
async fn issue_recovers_metadata_only_at_quota_and_revocation_is_immediate() -> TestResult {
    let (_dir, store, clock, actor, service) = fixture(TokenPolicy {
        max_active_per_principal: 1,
        ..Default::default()
    })
    .await?;
    let first = service.issue(&actor, true, body(), "one").await?;
    let raw = first.secret.ok_or("missing secret")?;
    assert_eq!(raw.expose().len(), "shaula_pat_v1_".len() + 76);
    let (identity, id) = service.authenticate(raw.expose()).await?;
    assert_eq!(identity.name, actor.name);
    assert_eq!(identity.scopes, vec![Scope::FleetRead]);
    let recovered = service.issue(&actor, true, body(), "one").await?;
    assert!(recovered.secret.is_none());
    assert_eq!(recovered.access_token.id, id);
    assert!(matches!(
        service.issue(&actor, true, body(), "two").await,
        Err(TokenError::Quota)
    ));
    let mut changed = body();
    changed.name = "other".into();
    assert!(matches!(
        service.issue(&actor, true, changed, "one").await,
        Err(TokenError::Conflict)
    ));
    assert!(matches!(
        service.get("other", &id).await,
        Err(TokenError::NotFound)
    ));
    assert!(matches!(
        service.revoke(&actor, &id, Some(2)).await,
        Err(TokenError::PreconditionFailed)
    ));
    assert_eq!(service.get(&actor.name, &id).await?.revision, 1);
    service.revoke(&actor, &id, Some(1)).await?;
    assert!(matches!(
        service.authenticate(raw.expose()).await,
        Err(TokenError::Unauthorized)
    ));
    let record = TokenStore::get(&store, &id).await?.ok_or("missing row")?;
    assert_eq!(record.revision, 2);
    assert_ne!(record.secret_digest.as_slice(), raw.expose().as_bytes());
    assert!(!serde_json::to_string(&record)?.contains(raw.expose()));
    clock.0.fetch_add(60_000, Ordering::SeqCst);
    let replay = service.issue(&actor, true, body(), "one").await?;
    assert_eq!(replay.access_token.state, "revoked");
    Ok(())
}

#[tokio::test]
async fn primary_scope_expiry_policy_and_realm_are_enforced() -> TestResult {
    let (_dir, store, clock, actor, service) = fixture(Default::default()).await?;
    assert!(matches!(
        service.issue(&actor, false, body(), "pat").await,
        Err(TokenError::PrimaryRequired)
    ));
    let mut forbidden = body();
    forbidden.scopes = vec!["access-token.write".into()];
    assert!(matches!(
        service.issue(&actor, true, forbidden, "write").await,
        Err(TokenError::NonDelegable)
    ));
    let issued = service.issue(&actor, true, body(), "one").await?;
    let raw = issued.secret.ok_or("secret")?;
    let revoked_policy = PersonalTokens::new(
        Arc::new(store.clone()),
        clock.clone(),
        Default::default(),
        "realm".into(),
        Default::default(),
    )?;
    assert!(matches!(
        revoked_policy.authenticate(raw.expose()).await,
        Err(TokenError::Unauthorized)
    ));
    let reduced = PersonalTokens::new(
        Arc::new(store.clone()),
        clock.clone(),
        Default::default(),
        "realm".into(),
        [(actor.name.clone(), vec![])].into(),
    )?;
    assert!(reduced
        .authenticate(raw.expose())
        .await?
        .0
        .scopes
        .is_empty());
    let other_realm = PersonalTokens::new(
        Arc::new(store.clone()),
        clock.clone(),
        Default::default(),
        "other".into(),
        [(actor.name.clone(), actor.scopes.clone())].into(),
    )?;
    assert!(matches!(
        other_realm.authenticate(raw.expose()).await,
        Err(TokenError::Unauthorized)
    ));
    clock.0.fetch_add(60_000, Ordering::SeqCst);
    assert!(matches!(
        service.authenticate(raw.expose()).await,
        Err(TokenError::Unauthorized)
    ));
    assert_eq!(
        service
            .get(&actor.name, &issued.access_token.id)
            .await?
            .state,
        "expired"
    );
    Ok(())
}

#[tokio::test]
async fn concurrent_issuance_has_one_secret_and_cursor_is_owner_bound() -> TestResult {
    let (_dir, _store, _clock, actor, service) = fixture(Default::default()).await?;
    let (a, b) = tokio::join!(
        service.issue(&actor, true, body(), "same"),
        service.issue(&actor, true, body(), "same")
    );
    let (a, b) = (a?, b?);
    assert_eq!(a.access_token.id, b.access_token.id);
    assert_ne!(a.secret.is_some(), b.secret.is_some());
    service.issue(&actor, true, body(), "second").await?;
    let page = service
        .list(
            &actor.name,
            TokenQuery {
                limit: Some(1),
                ..Default::default()
            },
        )
        .await?;
    let cursor = page.next_cursor.ok_or("cursor")?;
    assert!(matches!(
        service
            .list(
                "other",
                TokenQuery {
                    cursor: Some(cursor.clone()),
                    ..Default::default()
                }
            )
            .await,
        Err(TokenError::Invalid)
    ));
    let second = service
        .list(
            &actor.name,
            TokenQuery {
                cursor: Some(cursor),
                ..Default::default()
            },
        )
        .await?;
    assert_eq!(second.items.len(), 1);
    assert_ne!(page.items[0].id, second.items[0].id);
    Ok(())
}
