//! Real artifact and Profile Registry setup for input-contract HTTP tests.

use std::io::{Read, Write};

use axum::body::Body;
use axum::http::StatusCode;
use serde_json::{json, Value};
use sha2::Digest;
use tower::ServiceExt;

use crate::common::{attestation_harness::put_template_profile, authorized, fixture_artifact};

pub async fn publish(app: &axum::Router, key: &str, schema: &str, policy: Value) -> String {
    let (_, original) = fixture_artifact();
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(original.as_slice()));
    let mut builder = tar::Builder::new(Vec::new());
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        let path = entry.path().unwrap().into_owned();
        let mut content = Vec::new();
        entry.read_to_end(&mut content).unwrap();
        if path == std::path::Path::new("schemas/parameters.schema.json") {
            content = schema.as_bytes().to_vec();
        }
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, path, content.as_slice())
            .unwrap();
    }
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(&builder.into_inner().unwrap()).unwrap();
    let bytes = encoder.finish().unwrap();
    let digest = format!("sha256:{}", hex::encode(sha2::Sha256::digest(&bytes)));
    let mut upload = authorized("PUT", &format!("/api/v1/template-artifacts/{digest}"), None);
    *upload.body_mut() = Body::from(bytes);
    assert_eq!(
        app.clone().oneshot(upload).await.unwrap().status(),
        StatusCode::CREATED
    );
    let payload = json!({"artifact_digest":digest,"engine_ref":"terraform",
        "bindings":{"namespace":"runners","credential":"BINDING_SECRET_MUST_NOT_LEAK"},
        "fleet_input_policy":policy})
    .to_string();
    assert_eq!(
        put_template_profile(app, key, payload).await,
        StatusCode::ACCEPTED
    );
    digest
}

pub async fn response(app: &axum::Router, uri: &str) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(authorized("GET", uri, None))
        .await
        .unwrap();
    assert_eq!(response.headers()["cache-control"], "private, no-store");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 2 << 20)
        .await
        .unwrap();
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(!text.contains("BINDING_SECRET_MUST_NOT_LEAK"));
    assert!(!text.contains("bindings"));
    (status, serde_json::from_slice(&bytes).unwrap())
}
