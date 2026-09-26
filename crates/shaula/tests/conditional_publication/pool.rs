use axum::http::StatusCode;

use super::{common, support::*};

#[tokio::test]
async fn pool_noop_replays_original_result_after_head_advances() -> TestResult {
    let (app, store, _) = common::build_app_with_scan().await;
    common::attestation_harness::seed_profile(&app, &store, "k8s-linux", true).await;
    let (etag, _) = accepted(
        put(&app, POOL_URI, POOL_BODY, None, "create").await?,
        StatusCode::ACCEPTED,
    )
    .await?;
    let original = accepted(
        put(&app, POOL_URI, POOL_BODY, Some(&etag), "noop").await?,
        StatusCode::OK,
    )
    .await?;
    assert_eq!(original.1["noOp"], true);
    assert_eq!(original.1["revision"], 1);
    let changed = POOL_BODY.replace("\"weight\":20", "\"weight\":30");
    accepted(
        put(&app, POOL_URI, &changed, Some(&etag), "replace").await?,
        StatusCode::ACCEPTED,
    )
    .await?;
    let replay = accepted(
        put(&app, POOL_URI, POOL_BODY, Some(&etag), "noop").await?,
        StatusCode::OK,
    )
    .await?;
    assert_eq!(replay, original);
    let head = common::get_json(&app, POOL_URI).await;
    assert_eq!(head["metadata"]["revision"], 2);
    Ok(())
}
