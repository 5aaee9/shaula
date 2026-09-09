//! Minimal projection of Terraform 1.9.8 Docker 3.0.2/Kubernetes 2.33.0 plans.
//! Computed unknown fields and provider null/empty defaults are intentional.
use super::*;
use serde_json::json;
use shaula_core::template::{BindingsDigest, GenerationIdentity, ShaulaInputEnvelope};
use std::time::Duration;

pub(super) type TestResult = Result<(), Box<dyn std::error::Error>>;

pub(super) fn fixture(
    platform: &str,
) -> Result<(ProfileManifest, TemplateCreateRequest, Value), Box<dyn std::error::Error>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../templates")
        .join(platform)
        .join("profile.yaml");
    let manifest = crate::manifest::parse_manifest(&std::fs::read_to_string(path)?)?;
    let image = manifest
        .runner_image_digests
        .first()
        .ok_or("image missing")?;
    let mut input = ShaulaInputEnvelope::new(
        GenerationIdentity {
            fleet_key: "fleet".into(),
            scale_set_id: 1,
            id: "50a2dd2d-e48e-4b23-88de-b4bca9cf3b90".into(),
            runner_name: "runner".into(),
            generation_name: "generation".into(),
        },
        "frozen-jit".into(),
        BindingsDigest("commitment".into()),
    );
    input.bindings.insert("namespace".into(), json!("runners"));
    let request = TemplateCreateRequest {
        workspace_path: "workspace".into(),
        artifact_dir: "artifact".into(),
        pinned_artifact_digest: "digest".into(),
        expected_bindings_digest: input.bindings_digest.clone(),
        input,
        managed_shape: Vec::new(),
        environment: Vec::new(),
        timeout: Duration::from_secs(60),
        apply_intent_sink: None,
    };
    let plan = if platform == "docker" {
        docker(image, &request)
    } else {
        kubernetes(image, &request)
    };
    Ok((manifest, request, plan))
}

fn change(kind: &str, provider: &str, after: Value, unknown: Value) -> Value {
    json!({"mode":"managed","type":kind,"provider_name":provider,
        "change":{"actions":["create"],"after":after,"after_unknown":unknown}})
}

fn docker(image: &str, request: &TemplateCreateRequest) -> Value {
    let short: String = request.input.generation.id.chars().take(24).collect();
    let after = json!({"start":false,"must_run":false,"rm":false,"restart":"no","wait":false,
        "name":format!("shaula-fleet-{short}"),"command":[super::super::super::LISTENER,"run"],
        "env":["ACTIONS_RUNNER_INPUT_JITCONFIG=frozen-jit"], "image":"sha256:resolved-image-id",
        "privileged":null,"upload":[],"mounts":[],"volumes":[],"devices":[],"capabilities":[],
        "labels":[{"label":"shaula.fleet","value":"fleet"},
            {"label":"shaula.generation","value":request.input.generation.id}]});
    json!({"configuration":{"root_module":{"resources":[]}},
    "resource_changes":[change("docker_container","registry.terraform.io/kreuzwerker/docker",after,
        json!({"id":true,"entrypoint":true,"env":[false],"command":[false,false],"labels":[{},{}]}))],
    "prior_state":{"values":{"root_module":{"resources":[{
        "mode":"data","type":"docker_image","provider_name":"registry.terraform.io/kreuzwerker/docker",
        "values":{"name":image,"id":"sha256:resolved-image-id"}
    }]}}}})
}

fn kubernetes(image: &str, request: &TemplateCreateRequest) -> Value {
    let metadata = json!([{"name":"generation","namespace":"runners","generate_name":null,
        "labels":{"shaula.io/fleet":"fleet","shaula.io/generation-id":request.input.generation.id}}]);
    let secret = json!({"metadata":metadata,"type":"Opaque","immutable":false,
        "data":{"jit_config":"frozen-jit"},"binary_data":null});
    let pod = json!({"metadata":metadata,"target_state":["Pending"],"spec":[{
        "restart_policy":"Never","automount_service_account_token":false,"init_container":[],
        "container":[{"name":"runner","image":image,"command":[super::super::super::LISTENER,"run"],
            "args":null,"env_from":[],"lifecycle":[],"volume_device":[],
            "env":[{"name":"ACTIONS_RUNNER_INPUT_JITCONFIG","value":null,"value_from":[{
                "config_map_key_ref":[],"field_ref":[],"resource_field_ref":[],
                "secret_key_ref":[{"name":"generation","key":"jit_config","optional":false}]}]}],
            "volume_mount":[{"name":"setup-info","mount_path":"/home/runner/.setup_info",
                "sub_path":".setup_info","read_only":true}]}],
        "volume":[{"name":"setup-info","host_path":[],"projected":[],"secret":[{
            "secret_name":"generation","optional":false,"items":[{"key":".setup_info","path":".setup_info","mode":null}]}]}]
    }]});
    let computed = json!({"metadata":[{"uid":true,"resource_version":true,"labels":{}}],"id":true,
        "spec":[{"container":[{"image_pull_policy":true,"resources":[{"limits":true}]}]}]});
    json!({"configuration":{"root_module":{"resources":[]}},"resource_changes":[
        change("kubernetes_pod_v1","registry.terraform.io/hashicorp/kubernetes",pod,computed),
        change("kubernetes_secret_v1","registry.terraform.io/hashicorp/kubernetes",secret,
            json!({"metadata":[{"uid":true,"resource_version":true,"labels":{}}],"data":{},"id":true}))
    ]})
}
