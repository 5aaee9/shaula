use axum::{
    http::{HeaderMap, StatusCode, Uri},
    response::IntoResponse,
    Json,
};
use shaula_client::{types::Document, ChangeKind, Client, Error, MutationOptions, Secret};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
type Result = std::result::Result<(), Box<dyn std::error::Error>>;
#[tokio::test]
async fn old_oidc_server_supports_business_writes_without_guessing_identity() -> Result {
    let app = axum::Router::new().fallback(|uri: Uri| async move {
        if uri.path() == "/api/v1/session" {
            Json(serde_json::json!({"name":"display name","scopes":["fleet.write"]}))
        } else {
            Json(serde_json::json!({"changeId":"c","state":"Pending","revision":1}))
        }
    });
    let (client, task) = serve(app).await?;
    let attempt = client
        .fleets()
        .put(
            "key",
            &Document::parse("{}".into())?,
            MutationOptions::create(),
        )
        .await?;
    assert_eq!(client.execute_mutation(&attempt).await?.data.change_id, "c");
    let changed = client.with_credential(Secret::new("different-oidc-credential".into()))?;
    assert!(matches!(
        changed.execute_mutation(&attempt).await,
        Err(Error::Invalid(_))
    ));
    let pat = client.with_credential(Secret::new("shaula_pat_v1_not-supported".into()))?;
    assert!(matches!(
        pat.fleets()
            .put(
                "key",
                &Document::parse("{}".into())?,
                MutationOptions::create()
            )
            .await,
        Err(Error::Unsupported)
    ));
    task.abort();
    Ok(())
}
async fn serve(
    app: axum::Router,
) -> std::result::Result<(Client, tokio::task::JoinHandle<()>), Box<dyn std::error::Error>> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let client = Client::loopback(
        &format!("http://{}", listener.local_addr()?),
        Secret::new("credential".into()),
    )?;
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok((client, task))
}
#[tokio::test]
async fn read_retries_are_bounded_and_problem_details_are_sanitized() -> Result {
    let count = Arc::new(AtomicUsize::new(0));
    let counter = count.clone();
    let app=axum::Router::new().fallback(move || { let counter=counter.clone(); async move { counter.fetch_add(1,Ordering::SeqCst); (StatusCode::SERVICE_UNAVAILABLE,[("retry-after","0"),("x-request-id","not-a-valid-id")],Json(serde_json::json!({"status":503,"code":"AuthenticationUnavailable","detail":"secret-do-not-print"}))) } });
    let (client, task) = serve(app).await?;
    let error = client.session().await.err().ok_or("expected error")?;
    assert_eq!(count.load(Ordering::SeqCst), 3);
    assert!(
        matches!(&error,Error::Http {status:503,code,retry_after_seconds:Some(0),request_id:None} if code=="AuthenticationUnavailable")
    );
    assert!(!error.to_string().contains("secret-do-not-print"));
    task.abort();
    Ok(())
}
#[tokio::test]
async fn mutations_are_once_only_and_binding_rejects_another_principal() -> Result {
    let writes = Arc::new(AtomicUsize::new(0));
    let observed = writes.clone();
    let app=axum::Router::new().fallback(move |uri:Uri, headers:HeaderMap| {let writes=observed.clone(); async move {
        if uri.path()=="/api/v1/session" { return Json(serde_json::json!({"name":"operator","scopes":[],"principal":{"issuer":"https://id.test","subject":if headers.get("authorization").is_some_and(|v|v=="Bearer another") {"two"} else {"one"}}})).into_response(); }
        writes.fetch_add(1,Ordering::SeqCst); (StatusCode::SERVICE_UNAVAILABLE,Json(serde_json::json!({"code":"StorageUnavailable","status":503}))).into_response()
    }});
    let (client, task) = serve(app).await?;
    let attempt = client
        .fleets()
        .put(
            "key",
            &Document::parse("{}".into())?,
            MutationOptions::create(),
        )
        .await?;
    assert!(matches!(
        client.execute_mutation(&attempt).await,
        Err(Error::Http { status: 503, .. })
    ));
    assert_eq!(writes.load(Ordering::SeqCst), 1);
    let changed = client.with_credential(Secret::new("another".into()))?;
    assert!(matches!(
        changed.execute_mutation(&attempt).await,
        Err(Error::Invalid(_))
    ));
    assert_eq!(writes.load(Ordering::SeqCst), 1);
    task.abort();
    Ok(())
}
#[tokio::test]
async fn redirects_weak_versions_and_unknown_change_states_are_not_success() -> Result {
    let app=axum::Router::new().fallback(|uri:Uri| async move {
        if uri.path().ends_with("/redirect") { return (StatusCode::FOUND,[("location","https://other.example/")]).into_response(); }
        if uri.path().contains("-changes/") { return Json(serde_json::json!({"id":"change","resourceKind":"fleet","resourceKey":"key","revision":1,"kind":"Update","state":"FutureState","reason":"not complete"})).into_response(); }
        ([("etag","\"strong-fallback\""),("shaula-resource-version","W/\"weak\"")],Json(serde_json::json!({}))).into_response()
    });
    let (client, task) = serve(app).await?;
    assert!(client.fleets().get("key").await?.version.is_none());
    assert!(matches!(
        client.fleets().get("redirect").await,
        Err(Error::Http { status: 302, .. })
    ));
    assert!(matches!(
        client
            .wait(
                ChangeKind::Fleet,
                "change",
                std::time::Duration::from_millis(30)
            )
            .await,
        Err(Error::Tracking(_))
    ));
    task.abort();
    Ok(())
}
