//! Fleet admission must decode the active auth revision's policy version.

use super::*;
use serde_json::{json, Value};

type TestResult = Result<(), Box<dyn std::error::Error>>;

async fn active_v2_fixture(
) -> Result<(axum::Router, Arc<SqliteControlPlane>), Box<dyn std::error::Error>> {
    let (app, store, _) = build_app_with_scan().await;
    common::attestation_harness::seed_profile(&app, &store, "k8s-linux", true).await;
    let response = app
        .clone()
        .oneshot(authorized(
            "PUT",
            "/api/v1/github-auth-profiles/shared-github",
            Some(V2_PUT_BODY.into()),
        ))
        .await?;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    store
        .auth_apply_validation_v2(
            "shared-github",
            1,
            true,
            None,
            5,
            Some(promotion_now(&store).await),
        )
        .await?;
    Ok((app, store))
}

fn fleet_body(target: Value) -> Result<String, serde_json::Error> {
    let mut body: Value = serde_json::from_str(FLEET_BODY)?;
    body["github"]["auth_profile_ref"] = json!("shared-github");
    body["github"]["target"] = target;
    serde_json::to_string(&body)
}

#[tokio::test]
async fn active_v2_personal_repository_policy_allows_fleet_admission() -> TestResult {
    let (app, _store) = active_v2_fixture().await?;
    let response = app
        .clone()
        .oneshot(authorized(
            "PUT",
            "/api/v1/fleets/personal-repo",
            Some(fleet_body(
                json!({"kind": "repository", "owner": "5aaee9", "repository": "shaula"}),
            )?),
        ))
        .await?;
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 1 << 20).await?;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "{}",
        String::from_utf8_lossy(&body)
    );

    let view = get_json(&app, "/api/v1/fleets/personal-repo").await;
    assert_eq!(
        view["resolved"]["authDesired"]["profileKey"],
        "shared-github"
    );
    assert_eq!(view["resolved"]["authDesired"]["revision"], 1);

    let denied = app
        .clone()
        .oneshot(authorized(
            "PUT",
            "/api/v1/fleets/unknown-repo",
            Some(fleet_body(json!({
                "kind": "repository",
                "owner": "someone-else",
                "repository": "shaula"
            }))?),
        ))
        .await?;
    assert_eq!(denied.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let denied_body = axum::body::to_bytes(denied.into_body(), 1 << 20).await?;
    let denied_json: Value = serde_json::from_slice(&denied_body)?;
    assert_eq!(denied_json["code"], "Unprocessable");
    assert!(denied_json["detail"]
        .as_str()
        .is_some_and(|detail| detail.contains("does not cover")));
    Ok(())
}
