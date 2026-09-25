use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    Json,
};
use shaula_client::{
    types::{Document, IssueToken},
    ChangeKind, Client, MutationOptions, ResourceVersion, Secret,
};
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
#[derive(Clone, Default)]
struct Calls(Arc<Mutex<Vec<Call>>>);
type Call = (String, String, HeaderMap, Vec<u8>);
fn token() -> serde_json::Value {
    serde_json::json!({"id":"key","name":"test","scopes":[],"effective_scopes":[],"created_at":"2026-09-25T00:00:00Z","expires_at":"2026-09-26T00:00:00Z","last_used_at":null,"revoked_at":null,"state":"active","revision":1})
}
async fn handler(
    State(calls): State<Calls>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    bytes: Bytes,
) -> Response {
    let Ok(mut observed) = calls.0.lock() else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    observed.push((
        method.to_string(),
        uri.path().into(),
        headers,
        bytes.to_vec(),
    ));
    let path = uri.path();
    let change = serde_json::json!({"id":"key","resource_kind":"fleet","resource_key":"key","revision":1,"kind":"Update","state":"Converged","reason":null});
    let value = if path == "/api/v1/session" {
        serde_json::json!({"name":"operator","principal":{"issuer":"https://id.test","subject":"one"},"scopes":["fleet.read"]})
    } else if path == "/api/v1/access-tokens" && method == Method::POST {
        return (StatusCode::CREATED, Json(serde_json::json!({"access_token":token(),"secret_available":true,"token":"shaula_pat_v1_test"}))).into_response();
    } else if path.starts_with("/api/v1/access-tokens/") {
        if method == Method::DELETE {
            return StatusCode::NO_CONTENT.into_response();
        }
        token()
    } else if path == "/api/v1/access-tokens" {
        serde_json::json!({"items":[token()],"next_cursor":null})
    } else if path.contains("-changes/") {
        change
    } else if path.ends_with("/finalize") {
        let mut change = change;
        change["state"] = "Succeeded".into();
        serde_json::json!({"etag":"opaque","change":change,"no_op":false})
    } else if method != Method::GET {
        serde_json::json!({"changeId":"key","state":"Pending","revision":1,"noOp":false})
    } else {
        serde_json::json!({"items":[],"next_cursor":null})
    };
    ([("etag", "\"version\"")], Json(value)).into_response()
}
async fn fixture() -> Result<(Client, Calls, tokio::task::JoinHandle<()>)> {
    let calls = Calls::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let client = Client::loopback(
        &format!("http://{}", listener.local_addr()?),
        Secret::new("test-credential".into()),
    )?;
    let app = axum::Router::new()
        .fallback(handler)
        .with_state(calls.clone());
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok((client, calls, task))
}
#[tokio::test]
async fn every_inventory_route_is_exercised_through_public_sdk_methods() -> Result {
    let (client, calls, task) = fixture().await?;
    let doc = Document::parse("{}".into())?;
    let update = || -> Result<MutationOptions> {
        Ok(MutationOptions::update(ResourceVersion::parse(
            "\"version\"",
        )?))
    };
    client.session().await?;
    client.live().await?;
    client.ready().await?;
    macro_rules! resource {
        ($api:expr) => {{
            let api = $api;
            api.list().await?;
            api.get("key").await?;
            let a = api.put("key", &doc, MutationOptions::create()).await?;
            client.execute_mutation(&a).await?;
            let a = api.retire("key", update()?).await?;
            client.execute_mutation(&a).await?;
        }};
    }
    resource!(client.fleets());
    resource!(client.pools());
    resource!(client.templates());
    resource!(client.auth_profiles());
    client.fleets().status("key").await?;
    for kind in [
        ChangeKind::Fleet,
        ChangeKind::TemplatePool,
        ChangeKind::Profile,
    ] {
        client.change(kind, "key").await?;
    }
    client.templates().sources().await?;
    client.templates().variables("key").await?;
    client.templates().upload("key", vec![1, 2, 3]).await?;
    let a = client.templates().update("key", &doc, update()?).await?;
    client.execute_mutation(&a).await?;
    client.templates().revision("key", 1).await?;
    client.templates().input_contract("key", 1).await?;
    client.templates().attestation("key", 1, "key").await?;
    let a = client
        .templates()
        .attest("key", 1, "key", &doc, MutationOptions::create())
        .await?;
    client.execute::<Document>(&a).await?;
    client.auth_profiles().status("key").await?;
    client.auth_profiles().impact("key").await?;
    client.auth_profiles().installation_link("key").await?;
    client.auth_profiles().revision("key", 1).await?;
    let a = client
        .auth_profiles()
        .policy_update("key", &doc, update()?)
        .await?;
    client.execute_mutation(&a).await?;
    client.jobs(&vec![]).await?;
    client.job("key").await?;
    client.generations(&vec![]).await?;
    client.generation("key").await?;
    client.invocations("key", &vec![]).await?;
    client.logs("key", &vec![]).await?;
    let a = client.finalize("key", "verified absent", None).await?;
    client.execute_finalize(&a).await?;
    let api = client.access_tokens();
    api.list(&vec![]).await?;
    api.get("key").await?;
    api.current().await?;
    api.issue(
        &IssueToken {
            name: "test".into(),
            scopes: vec![],
            expires_in_seconds: Some(60),
        },
        "key".into(),
    )
    .await?;
    api.revoke("key", ResourceVersion::parse("\"version\"")?)
        .await?;
    api.revoke_current().await?;
    let inventory: serde_json::Value = serde_json::from_str(include_str!(
        "../../../docs/design/0039-route-inventory.json"
    ))?;
    let expected: BTreeSet<(String, String)> = inventory["operations"]
        .as_array()
        .ok_or("operations")?
        .iter()
        .map(|op| {
            let path = op["path"]
                .as_str()
                .unwrap_or_default()
                .split('/')
                .map(|s| {
                    if s == "{revision}" {
                        "1"
                    } else if s.starts_with('{') {
                        "key"
                    } else {
                        s
                    }
                })
                .collect::<Vec<_>>()
                .join("/");
            (op["method"].as_str().unwrap_or_default().into(), path)
        })
        .collect();
    let actual: BTreeSet<_> = calls
        .0
        .lock()
        .map_err(|_| "calls")?
        .iter()
        .map(|(m, p, _, _)| (m.clone(), p.clone()))
        .collect();
    assert_eq!(actual, expected);
    assert_eq!(actual.len(), 49);
    task.abort();
    Ok(())
}

#[tokio::test]
async fn explicit_replay_keeps_bytes_version_key_and_never_exposes_body_in_debug() -> Result {
    let (client, calls, task) = fixture().await?;
    let raw = r#"{"capacity":{"min_runners":0,"max_runners":2},"template_inputs":{"n":18446744073709551617,"ratio":1.2300000000000000001},"secret":"do-not-log"}"#;
    let body = Document::parse(raw.into())?;
    let attempt = client
        .fleets()
        .put(
            "key",
            &body,
            MutationOptions::update(ResourceVersion::parse("\"reviewed\"")?),
        )
        .await?;
    client.execute_mutation(&attempt).await?;
    client.execute_mutation(&attempt).await?;
    let calls = calls.0.lock().map_err(|_| "calls")?;
    let writes: Vec<_> = calls.iter().filter(|(m, _, _, _)| m == "PUT").collect();
    assert_eq!(writes.len(), 2);
    for (_, _, h, b) in &writes {
        assert_eq!(b.as_slice(), raw.as_bytes());
        assert_eq!(h["if-match"], "\"reviewed\"");
        assert_eq!(h["idempotency-key"], attempt.idempotency_key());
    }
    assert!(!format!("{attempt:?}{client:?}{body:?}").contains("do-not-log"));
    task.abort();
    Ok(())
}

#[test]
fn origins_and_versions_fail_closed() {
    for origin in [
        "http://example.com",
        "https://user:password@example.com",
        "https://example.com/api",
        "https://example.com?x=1",
        " https://example.com",
    ] {
        assert!(Client::new(origin, Secret::new("token".into())).is_err());
    }
    for version in ["*", "W/\"v\"", "v", "\"a\",\"b\"", "\"a b\""] {
        assert!(ResourceVersion::parse(version).is_err());
    }
}
