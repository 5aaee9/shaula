// Unit tests extracted to their own module to keep the host file
// within the 400-line limit (AGENTS.md).
use super::*;

fn manifest() -> ProfileManifest {
    serde_yaml::from_str(
        r#"
api_version: shaula.io/template-profile/v1
kind: RunnerTemplateProfile
platform: kubernetes
runtime:
  protocol: terraform-cli/v1
  engine: terraform
  root_module: .
  required_version: ">= 1.9, < 2.0"
bindings_contract: shaula.bindings.kubernetes/v1
schemas:
  bindings: schemas/bindings.schema.json
  parameters: schemas/parameters.schema.json
managed_resource_shape:
  - role: bootstrap
    terraform_type: kubernetes_secret_v1
    exact_count: 1
  - role: runner
    terraform_type: kubernetes_pod_v1
    exact_count: 1
runner_image_digests:
  - ghcr.io/actions/actions-runner:2.323.0@sha256:3f2a1b9c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f708192a3b4c5d6e7f8
runtime_policy_digest: sha256:policy-v1
"#,
    )
    .unwrap()
}

#[test]
fn manifest_validates() {
    assert!(manifest().validate().is_ok());
}

#[test]
fn platform_comes_only_from_manifest() {
    assert_eq!(manifest().platform(), TemplatePlatform::Kubernetes);
    assert_eq!(
        TemplatePlatform::from_manifest_value("unknown"),
        TemplatePlatform::Other
    );
}

#[test]
fn manifest_rejects_custom_engine() {
    let mut m = manifest();
    m.runtime.engine = "custom-exec".into();
    assert!(m.validate().is_err());
}

#[test]
fn manifest_rejects_duplicate_roles() {
    let m = ProfileManifest {
        managed_resource_shape: vec![
            ManagedResourceRole {
                role: "runner".into(),
                terraform_type: "t".into(),
                exact_count: 1,
            },
            ManagedResourceRole {
                role: "runner".into(),
                terraform_type: "t2".into(),
                exact_count: 1,
            },
        ],
        ..manifest()
    };
    assert!(m.validate().is_err());
}

#[test]
fn bindings_digest_is_keyed_not_plaintext_digest() {
    let d1 = BindingsDigest::from_keyed_material("rev-1", b"server-secret").unwrap();
    let d2 = BindingsDigest::from_keyed_material("rev-2", b"server-secret").unwrap();
    let d3 = BindingsDigest::from_keyed_material("rev-1", b"other-secret").unwrap();
    assert_ne!(d1, d2, "different revisions produce different commitments");
    assert_ne!(
        d1, d3,
        "commitment depends on the server key, not just plaintext"
    );
    assert!(d1.0.starts_with("bd1_"));
}

#[test]
fn tfvars_has_single_shaula_variable() {
    let env = ShaulaInputEnvelope::new(
        GenerationIdentity {
            fleet_key: "fleet-a".into(),
            scale_set_id: 123,
            id: "gen-1".into(),
            runner_name: "runner-1".into(),
            generation_name: "s0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .into(),
        },
        "jit-secret".into(),
        BindingsDigest("bd1_x".into()),
    );
    let doc: serde_json::Value = serde_json::from_str(&env.to_tfvars().unwrap()).unwrap();
    let obj = doc.as_object().unwrap();
    assert_eq!(
        obj.len(),
        1,
        "tfvars must have exactly one top-level variable"
    );
    assert!(obj.contains_key("shaula"));
    assert_eq!(obj["shaula"]["generation"]["id"], "gen-1");
}

#[test]
fn result_validation_rejects_mismatches() {
    let shape = vec![
        ManagedResourceRole {
            role: "bootstrap".into(),
            terraform_type: "s".into(),
            exact_count: 1,
        },
        ManagedResourceRole {
            role: "runner".into(),
            terraform_type: "p".into(),
            exact_count: 1,
        },
    ];
    let digest = BindingsDigest("bd1_x".into());
    let mut result = ShaulaResultEnvelope {
        contract_version: 1,
        generation_id: "gen-1".into(),
        bindings_digest: digest.clone(),
        resources: vec![
            ResultResource {
                role: "bootstrap".into(),
                id: "opaque-1".into(),
                incarnation: None,
            },
            ResultResource {
                role: "runner".into(),
                id: "opaque-2".into(),
                incarnation: None,
            },
        ],
    };
    assert!(result.validate_against("gen-1", &digest, &shape).is_ok());

    result.generation_id = "gen-other".into();
    assert!(result.validate_against("gen-1", &digest, &shape).is_err());
    result.generation_id = "gen-1".into();

    result.bindings_digest = BindingsDigest("bd1_other".into());
    assert!(result.validate_against("gen-1", &digest, &shape).is_err());
    result.bindings_digest = digest.clone();

    result.resources.truncate(1);
    assert!(result.validate_against("gen-1", &digest, &shape).is_err());
}

#[test]
fn result_rejects_unknown_fields() {
    let raw = r#"{"contract_version":1,"generation_id":"g","bindings_digest":"bd1_x","resources":[],"extra":1}"#;
    let parsed: Result<ShaulaResultEnvelope, _> = serde_json::from_str(raw);
    assert!(parsed.is_err());
}
