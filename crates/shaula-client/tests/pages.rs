use axum::{extract::Query, Json};
use shaula_client::{Client, Error, Secret};
use std::collections::HashMap;
type Result = std::result::Result<(), Box<dyn std::error::Error>>;

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

fn keys(list: &serde_json::Value, field: &str) -> Vec<String> {
    list[field]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v["key"].as_str().map(str::to_owned))
        .collect()
}

#[tokio::test]
async fn complete_lists_follow_every_cursor_and_stay_compatible() -> Result {
    let app = axum::Router::new()
        .route(
            "/api/v1/fleets",
            axum::routing::get(|Query(q): Query<HashMap<String, String>>| async move {
                assert!(q.contains_key("limit"), "list reads always bound the page");
                Json(match q.get("cursor").map(String::as_str) {
                    None => {
                        serde_json::json!({"fleets":[{"key":"a","revision":1,"incarnation":"i"},{"key":"b","revision":1,"incarnation":"i"}],"next_cursor":"c1"})
                    }
                    Some("c1") => serde_json::json!({"fleets":[{"key":"c","revision":1,"incarnation":"i"}],"next_cursor":null}),
                    Some(_) => serde_json::json!({"fleets":[]}),
                })
            }),
        )
        // An older server ignores the query and returns one unpaginated list.
        .route(
            "/api/v1/template-profiles",
            axum::routing::get(|| async { Json(serde_json::json!({"profiles":[{"key":"t"}]})) }),
        );
    let (client, task) = serve(app).await?;
    let fleets: serde_json::Value = client.fleets().list().await?.data.decode()?;
    assert_eq!(keys(&fleets, "fleets"), ["a", "b", "c"]);
    assert!(fleets["next_cursor"].is_null());
    let typed = client.fleets().list_typed().await?;
    assert_eq!(typed.data.fleets.len(), 3);
    let page: serde_json::Value = client
        .fleets()
        .list_page(Some("c1"), Some(1))
        .await?
        .data
        .decode()?;
    assert_eq!(keys(&page, "fleets"), ["c"]);
    let profiles: serde_json::Value = client.templates().list().await?.data.decode()?;
    assert_eq!(keys(&profiles, "profiles"), ["t"]);
    task.abort();
    Ok(())
}

#[tokio::test]
async fn a_repeating_cursor_is_a_protocol_error() -> Result {
    let app = axum::Router::new().route(
        "/api/v1/fleets",
        axum::routing::get(|| async {
            Json(serde_json::json!({"fleets":[{"key":"a"}],"next_cursor":"loop"}))
        }),
    );
    let (client, task) = serve(app).await?;
    assert!(matches!(client.fleets().list().await, Err(Error::Protocol)));
    task.abort();
    Ok(())
}
