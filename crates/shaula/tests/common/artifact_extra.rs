use super::*;
pub fn fixture_artifact_docker() -> (String, Vec<u8>) {
    let manifest = "api_version: shaula.io/template-profile/v1\nkind: RunnerTemplateProfile\nplatform: docker\nruntime:\n  protocol: terraform-cli/v1\n  engine: terraform\n  root_module: .\n  required_version: \">= 1.9, < 2.0\"\nbindings_contract: shaula.bindings.docker/v1\ncontainer_bootstrap_contract: shaula.container-bootstrap/v1\nschemas:\n  bindings: schemas/bindings.schema.json\n  parameters: schemas/parameters.schema.json\nmanaged_resource_shape:\n  - role: runner\n    terraform_type: docker_container\n    exact_count: 1\nrunner_image_digests:\n  - ghcr.io/actions/actions-runner:2.323.0@sha256:3f2a1b9c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f708192a3b4c5d6e7f8\nruntime_policy_digest: sha256:policy-v1\n";
    let mut builder = tar::Builder::new(Vec::new());
    for (name, content) in [
        ("profile.yaml", manifest),
        (".terraform.lock.hcl", FIXTURE_LOCK_HCL),
        ("schemas/bindings.schema.json", "{}"),
        ("schemas/parameters.schema.json", "{}"),
        ("main.tf", "resource \"docker_container\" \"runner\" {}\n"),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, name, content.as_bytes())
            .unwrap();
    }
    let tar_bytes = builder.into_inner().unwrap();
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    std::io::Write::write_all(&mut encoder, &tar_bytes).unwrap();
    let bytes = encoder.finish().unwrap();
    let digest = {
        use sha2::Digest;
        let mut hasher = sha2::Sha256::new();
        hasher.update(&bytes);
        format!("sha256:{}", hex::encode(hasher.finalize()))
    };
    (digest, bytes)
}

/// A SECOND-revision template body: same artifact, semantically changed
/// input policy — identical content is a durable NO-OP under R9-02, so a
/// genuine revision 2 must change something.
pub fn template_put_body_r2(digest: &str) -> String {
    TEMPLATE_PUT_BODY.replace("PLACEHOLDER", digest).replace(
        "\"fleet_input_policy\": {\"size_class\": [\"standard\"]}",
        "\"fleet_input_policy\": {\"size_class\": [\"standard\", \"large\"]}",
    )
}

/// An attestation PUT request with its REQUIRED `If-None-Match: *`
/// precondition (R10-05, spec 0005 §5).
pub fn attestation_put_request(uri: &str, body: String) -> Request<Body> {
    let mut req = authorized("PUT", uri, Some(body));
    req.headers_mut()
        .insert("if-none-match", axum::http::HeaderValue::from_static("*"));
    req
}
