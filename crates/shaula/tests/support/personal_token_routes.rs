use super::*;

#[test]
fn management_router_and_inventory_are_bidirectional() -> TestResult {
    use std::collections::BTreeSet;
    let source = include_str!("../../../shaula-http/src/router/mod.rs");
    let inventory: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../docs/design/0039-route-inventory.json"
    ))?;
    let declared: BTreeSet<_> = inventory["operations"]
        .as_array()
        .ok_or("operations")?
        .iter()
        .map(|op| {
            format!(
                "{} {}",
                op["method"].as_str().unwrap_or(""),
                op["path"].as_str().unwrap_or("")
            )
        })
        .collect();
    let mut registered = BTreeSet::new();
    // The management router keeps literal route declarations in this module.
    // Fail on an unfamiliar registration shape rather than silently omitting it.
    for route in source.split(".route(").skip(1) {
        let (path, handlers) = route
            .trim_start()
            .strip_prefix('"')
            .ok_or("route must be literal")?
            .split_once('"')
            .ok_or("route path")?;
        if matches!(
            path,
            "/auth/oidc/login" | "/auth/oidc/callback" | "/auth/oidc/logout"
        ) {
            continue;
        }
        let mut count = 0;
        for method in ["get", "put", "post", "delete", "patch", "head", "options"] {
            if handlers.contains(&format!("{method}(")) {
                registered.insert(format!("{} {path}", method.to_uppercase()));
                count += 1;
            }
        }
        assert!(count > 0, "unclassified route {path}");
    }
    assert_eq!(
        registered, declared,
        "every management route needs an inventory entry and SDK/CLI coverage"
    );
    Ok(())
}

#[tokio::test]
async fn all_inventory_routes_require_identity_and_scoped_routes_reject_identity_only_pat(
) -> TestResult {
    let fixture = Fixture::new().await?;
    let TokenIssue::Issued { secret, .. } = fixture
        .primary("ops")?
        .access_tokens()
        .issue(&request(vec![]), "identity-only".into())
        .await?
    else {
        return Err("secret".into());
    };
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let inventory: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../docs/design/0039-route-inventory.json"
    ))?;
    let operations = inventory["operations"].as_array().ok_or("operations")?;
    assert_eq!(operations.len(), 49);
    for operation in operations {
        let method = operation["method"]
            .as_str()
            .ok_or("method")?
            .parse::<reqwest::Method>()?;
        let path = operation["path"]
            .as_str()
            .ok_or("path")?
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
        let url = format!("{}{path}", fixture.origin);
        assert_eq!(
            http.request(method.clone(), &url).send().await?.status(),
            StatusCode::UNAUTHORIZED,
            "{method} {path}"
        );
        if operation["scopes"].as_array().is_none_or(Vec::is_empty) {
            continue;
        }
        let response = http
            .request(method.clone(), url)
            .bearer_auth(secret.expose())
            .header("content-type", "application/json")
            .body(if path.ends_with("/finalize") {
                r#"{"reason":"verified absent"}"#
            } else {
                "{}"
            })
            .send()
            .await?;
        // Profile changes look up their resource kind before checking the corresponding scope.
        if path.starts_with("/api/v1/profile-changes/") {
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        } else {
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{method} {path}");
        }
    }
    Ok(())
}

#[tokio::test]
async fn legacy_idempotency_record_is_never_adopted_by_a_new_principal() -> TestResult {
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
    let fixture = Fixture::new().await?;
    let mut options = sea_orm::ConnectOptions::new(format!(
        "sqlite://{}",
        fixture
            ._dir
            .path()
            .join("db")
            .to_string_lossy()
            .replace('\\', "/")
    ));
    options.sqlx_logging(false);
    let db = sea_orm::Database::connect(options).await?;
    db.execute(Statement::from_string(DatabaseBackend::Sqlite,"INSERT INTO idempotency_records(id,resource_kind,resource_key,idempotency_key,request_hash,response_status,created_at) VALUES('legacy','github_auth_profile','old-app','old-key','unknown',202,1)".to_owned())).await?;
    let client = fixture.primary("ops")?;
    let body=Document::parse(r#"{"kind":"github_app","schema_version":2,"app_id":"42","private_key":"private","target_policy":[{"kind":"organization","owner":"example"}]}"#.into())?;
    let mut options = MutationOptions::create();
    options.idempotency_key = "old-key".into();
    let attempt = client
        .auth_profiles()
        .put("old-app", &body, options)
        .await?;
    assert!(
        matches!(client.execute_mutation(&attempt).await,Err(shaula_client::Error::Http {status:409,code,..}) if code=="LegacyIdempotencyConflict")
    );
    Ok(())
}
