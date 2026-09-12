use crate::{ForgejoClient, ForgejoError, ForgejoScope};
use axum::{
    body::Body,
    http::{Response, StatusCode},
    routing::any,
    Router,
};
use shaula_core::ports::{forgejo::ForgejoPoolPort, EffectOutcome};

type TestResult = Result<(), Box<dyn std::error::Error>>;
struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}
async fn serve(
    status: StatusCode,
    body: &'static str,
) -> Result<(Server, ForgejoClient), Box<dyn std::error::Error>> {
    let app = Router::new()
        .route(
            "/api/v1/version",
            axum::routing::get(|| async { axum::Json(serde_json::json!({"version":"16.0.4"})) }),
        )
        .fallback(any(move || async move {
            let mut response = Response::new(Body::from(body));
            *response.status_mut() = status;
            response
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}", listener.local_addr()?);
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok((
        Server(task),
        ForgejoClient::new(&url, "control-token", ForgejoScope::Instance)?,
    ))
}

#[tokio::test]
async fn successful_registration_with_unreadable_body_is_uncertain() -> TestResult {
    for body in ["", "{not-json", r#"{"id":42,"uuid":"runner","token":""}"#] {
        let (_server, client) = serve(StatusCode::CREATED, body).await?;
        let outcome = ForgejoPoolPort::register_runner(&client, "pool-generation", None)
            .await
            .map_err(|_| "registration should be classified as uncertain")?;
        assert!(matches!(outcome, EffectOutcome::Uncertain { .. }));
    }
    Ok(())
}

#[tokio::test]
async fn unexpected_registration_status_is_uncertain_without_leaking_response() -> TestResult {
    for status in [StatusCode::OK, StatusCode::BAD_GATEWAY] {
        let (_server, client) = serve(status, r#"{"token":"one-shot-secret"}"#).await?;
        let result = ForgejoPoolPort::register_runner(&client, "pool-g", None).await;
        assert!(matches!(result, Ok(EffectOutcome::Uncertain { .. })));
        assert!(!format!("{result:?}").contains("one-shot-secret"));
    }
    Ok(())
}

#[tokio::test]
async fn registration_body_is_bounded_before_network_effects() -> TestResult {
    let client = ForgejoClient::new(
        "http://127.0.0.1:1",
        "control-token",
        ForgejoScope::Instance,
    )?;
    assert!(matches!(
        client
            .register_runner("pool-g", Some(&"a".repeat(1025)))
            .await,
        Err(ForgejoError::Configuration(_))
    ));
    assert!(matches!(
        client.register_runner("pool-g\n", None).await,
        Err(ForgejoError::Configuration(_))
    ));
    assert!(ForgejoClient::new(
        "http://127.0.0.1:1",
        "a".repeat(4097),
        ForgejoScope::Instance
    )
    .is_err());
    Ok(())
}

#[tokio::test]
async fn repeated_inventory_page_fails_instead_of_hanging_or_proving_absence() -> TestResult {
    let (_server, client) = serve(StatusCode::OK,
        r#"[{"id":1,"uuid":"u","name":"pool-g","status":"idle","ephemeral":true,"labels":["linux"]}]"#).await?;
    let result =
        tokio::time::timeout(std::time::Duration::from_secs(2), client.list_runners()).await?;
    assert!(matches!(result, Err(ForgejoError::Unavailable { .. })));
    Ok(())
}

#[tokio::test]
async fn reused_scope_names_cannot_authorize_absence_or_mutation() -> TestResult {
    for scope in [
        ForgejoScope::User,
        ForgejoScope::Organization("team".into()),
        ForgejoScope::Repository {
            owner: "owner".into(),
            name: "repo".into(),
        },
    ] {
        let (_server, base) = serve(StatusCode::OK, r#"{"id":99}"#).await?;
        let client = ForgejoClient::new(base.instance_url().as_str(), "control-token", scope)?
            .with_expected_scope_identity(42)?;
        assert!(matches!(
            client.list_runners().await,
            Err(ForgejoError::PermissionDenied)
        ));
        assert!(matches!(
            client.get_runner(42).await,
            Err(ForgejoError::PermissionDenied)
        ));
        assert!(matches!(
            client.delete_runner(42).await,
            Err(ForgejoError::PermissionDenied)
        ));
        assert!(matches!(
            client.register_runner("pool-g", None).await,
            Err(ForgejoError::PermissionDenied)
        ));
    }
    Ok(())
}

#[test]
fn undeclared_runner_can_have_null_labels() -> TestResult {
    let runner: crate::Runner = serde_json::from_str(
        r#"{"id":1,"uuid":"u","name":"pool-g","status":"offline","ephemeral":true,"labels":null}"#,
    )?;
    assert!(runner.labels.is_empty());
    assert!(!runner.has_labels(&["linux".into()]));
    Ok(())
}

#[test]
fn label_names_are_unique_even_when_backend_targets_differ() {
    for labels in [
        vec!["linux:host".into(), "linux:docker://image".into()],
        vec!["linux,extra:host".into()],
    ] {
        assert!(crate::labels::normalize_label_names(&labels).is_err());
    }
}
