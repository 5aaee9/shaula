use super::*;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use shaula_core::template::{BindingsDigest, GenerationIdentity, ShaulaInputEnvelope};

const IMAGE: &str = "ghcr.io/actions/actions-runner:2.337.0@sha256:e5496277be5d09bc968b3d64911b74e219ac4a3f2edce956a3ecf9271bea1ef4";
const UID: &str = "ddbe9107-f968-414e-9446-5dabf9c27289";

fn request() -> TemplateCreateRequest {
    let mut input = ShaulaInputEnvelope::new(
        GenerationIdentity {
            fleet_key: "fleet".into(),
            scale_set_id: 1,
            id: "50a2dd2d-e48e-4b23-88de-b4bca9cf3b90".into(),
            runner_name: "runner".into(),
            generation_name: "generation".into(),
        },
        "private-jit".into(),
        BindingsDigest("commitment".into()),
    );
    input.bindings.insert("namespace".into(), json!("runners"));
    TemplateCreateRequest {
        workspace_path: "workspace".into(),
        artifact_dir: "artifact".into(),
        pinned_artifact_digest: "artifact-digest".into(),
        expected_bindings_digest: input.bindings_digest.clone(),
        input,
        managed_shape: Vec::new(),
        environment: Vec::new(),
        timeout: Duration::from_secs(60),
        apply_intent_sink: None,
    }
}

fn docker_fixture(request: &TemplateCreateRequest) -> Value {
    let short: String = request.input.generation.id.chars().take(24).collect();
    json!({
        "Id": "container-id", "Image": "image-id",
        "Name": format!("/shaula-fleet-{short}"), "Mounts": [],
        "Config": {
            "Cmd": [LISTENER, "run"], "Entrypoint": null, "User": "runner",
            "Env": [format!("ACTIONS_RUNNER_INPUT_JITCONFIG={}", request.input.jit_config)],
            "Labels": {"shaula.fleet": "fleet", "shaula.generation": request.input.generation.id}
        },
        "State": {"Status":"created", "Running":false, "Restarting":false, "Dead":false},
        "HostConfig": {"RestartPolicy":{"Name":"no"}, "AutoRemove":false,"Privileged":false}
    })
}

fn kubernetes_fixture(request: &TemplateCreateRequest, kind: &str) -> Value {
    json!({
        "kind": kind, "apiVersion":"v1",
        "metadata": {"uid":UID,"resourceVersion":"125","name":"generation","namespace":"runners",
            "labels":{"shaula.io/fleet":"fleet","shaula.io/generation-id":request.input.generation.id}},
        "type":"Opaque", "immutable":false,
        "data":{"jit_config":STANDARD.encode(&request.input.jit_config)},
        "status":{"phase":"Pending"},
        "spec":{
            "restartPolicy":"Never", "automountServiceAccountToken":false,
            "containers":[{"name":"runner", "image":IMAGE,"command":[LISTENER,"run"],
                "env":[{"name":"ACTIONS_RUNNER_INPUT_JITCONFIG", "valueFrom":{"secretKeyRef":{
                    "name":"generation", "key":"jit_config", "optional":false}}}],
                "volumeMounts":[{"name":"bootstrap","mountPath":"/home/runner/.setup_info",
                    "subPath":".setup_info","readOnly":true}]}],
            "volumes":[{"name":"bootstrap","secret":{"secretName":"generation", "optional":false,
                "items":[{"key":".setup_info","path":".setup_info"}]}}]
        }
    })
}

fn projection(detail: String) -> SetupProjection {
    SetupProjection {
        status: "ready".into(),
        invocation_id: Some("invocation".into()),
        content_version: None,
        detail: Some(detail),
        partial: false,
    }
}

#[test]
fn setup_render_preserves_json_text_and_bounds_serialized_bytes(
) -> Result<(), Box<dyn std::error::Error>> {
    let text = "Apply complete!\nquote: \"\n中文";
    let rendered: Value = serde_json::from_slice(&render(Some(projection(text.into()))))?;
    assert_eq!(rendered[0]["Group"], GROUP);
    assert_eq!(rendered[0]["Detail"], text);
    for detail in [
        "中".repeat(300_000),
        "\0".repeat(512 * 1024),
        "\\".repeat(490 * 1024),
        "line\n".repeat(21_000),
        "\n".repeat(21_000),
    ] {
        let encoded = render(Some(projection(detail)));
        assert!(encoded.len() + 64 * 1024 <= 1024 * 1024);
        let value: Value = serde_json::from_slice(&encoded)?;
        assert!(value[0]["Detail"]
            .as_str()
            .is_some_and(|detail| detail.lines().count() <= 20_000));
        assert!(value[0]["Detail"]
            .as_str()
            .is_some_and(|value| value.contains("truncated")));
    }
    Ok(())
}

#[test]
fn unavailable_projection_does_not_emit_unapproved_detail() -> Result<(), Box<dyn std::error::Error>>
{
    let mut denied = projection("do not expose this".into());
    denied.status = "withheld".into();
    let text = String::from_utf8(render(Some(denied)))?;
    assert!(!text.contains("do not expose"));
    assert!(text.contains("unavailable or withheld"));
    assert_eq!(render(None), text.as_bytes());
    Ok(())
}

#[test]
fn docker_refuses_replacement_or_already_started_containers() {
    let request = request();
    let original = docker_fixture(&request);
    assert!(docker::validate(&original, "container-id", "image-id", &request).is_ok());
    for (path, value) in [
        ("/Id", json!("replacement")),
        ("/Image", json!("other-image")),
        ("/Name", json!("/unrelated")),
        (
            "/Config/Labels/shaula.generation",
            json!("other-generation"),
        ),
        ("/Config/Cmd", json!(["/bin/sh", "-c", "sleep infinity"])),
        (
            "/Config/Env",
            json!(["ACTIONS_RUNNER_INPUT_JITCONFIG=wrong"]),
        ),
        ("/Config/User", json!("0:0")),
        ("/State/Status", json!("exited")),
        ("/State/Running", json!(true)),
        ("/HostConfig/Privileged", json!(true)),
        ("/Mounts", json!([{"Source":"/host"}])),
    ] {
        let mut changed = original.clone();
        if let Some(target) = changed.pointer_mut(path) {
            *target = value;
        }
        assert!(
            docker::validate(&changed, "container-id", "image-id", &request).is_err(),
            "{path}"
        );
    }
}

#[test]
fn slow_optional_delivery_reserves_time_for_identity_check_and_start() {
    assert_eq!(
        process::optional_budget(Duration::from_secs(60)),
        Duration::from_secs(5)
    );
    assert_eq!(
        process::optional_budget(Duration::from_secs(32)),
        Duration::from_secs(2)
    );
    assert_eq!(
        process::optional_budget(Duration::from_secs(25)),
        Duration::ZERO
    );
}

#[test]
fn docker_rejects_case_insensitive_runner_input_overrides() {
    let request = request();
    let mut container = docker_fixture(&request);
    container["Config"]["Env"] = json!([
        "ACTIONS_RUNNER_INPUT_JITCONFIG=private-jit",
        "actions_runner_input_jitconfig=override"
    ]);
    assert!(docker::validate(&container, "container-id", "image-id", &request).is_err());
}

#[test]
fn partial_projection_remains_visible_when_capture_has_a_gap(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut partial = projection("Apply complete!".into());
    partial.partial = true;
    let rendered = String::from_utf8(render(Some(partial)))?;
    assert!(rendered.contains("incomplete or truncated"));
    Ok(())
}

#[test]
fn docker_endpoint_cannot_select_remote_or_unauthenticated_transport() {
    assert!(docker::local_host("unix:///var/run/docker.sock"));
    assert!(docker::local_host("npipe:////./pipe/docker_engine"));
    for host in [
        "tcp://host:2375",
        "ssh://host",
        "npipe:////remote/pipe/docker_engine",
        "npipe:////./pipe/../elsewhere",
    ] {
        assert!(!docker::local_host(host), "{host}");
    }
}

#[test]
fn kubernetes_rejects_a_prior_listener_exit_even_when_pod_remains_pending() {
    let request = request();
    let mut pod = kubernetes_fixture(&request, "Pod");
    pod["status"]["containerStatuses"] = json!([{"state":{"terminated":{"exitCode":0}}}]);
    assert!(kubernetes::validate_pod(&pod, IMAGE, &request).is_err());
}

#[test]
fn kubernetes_requires_exact_resource_identity() {
    let request = request();
    let original = kubernetes_fixture(&request, "Secret");
    let resource = ResultResource {
        role: "bootstrap".into(),
        id: "runners/generation".into(),
        incarnation: Some(UID.into()),
    };
    assert!(kubernetes::validate_metadata(&original, &resource, &request, "Secret").is_ok());
    for (path, value) in [
        (
            "/metadata/uid",
            json!("516c57e8-7aa7-4b75-83b3-3afdd0e46ee9"),
        ),
        ("/metadata/name", json!("other")),
        ("/metadata/namespace", json!("other")),
        ("/metadata/labels/shaula.io~1generation-id", json!("other")),
        ("/metadata/resourceVersion", json!("")),
        ("/kind", json!("Pod")),
    ] {
        let mut changed = original.clone();
        if let Some(target) = changed.pointer_mut(path) {
            *target = value;
        }
        assert!(
            kubernetes::validate_metadata(&changed, &resource, &request, "Secret").is_err(),
            "{path}"
        );
    }
}

#[test]
fn kubernetes_refuses_missing_gate_or_early_started_listener() {
    let request = request();
    let original = kubernetes_fixture(&request, "Pod");
    assert!(kubernetes::validate_pod(&original, IMAGE, &request).is_ok());
    for (path, value) in [
        ("/status/phase", json!("Running")),
        ("/spec/containers/0/image", json!("unofficial/runner")),
        ("/spec/containers/0/command", json!(["bootstrap-shim"])),
        (
            "/spec/containers/0/env/0/valueFrom/secretKeyRef/name",
            json!("other"),
        ),
        ("/spec/containers/0/volumeMounts/0/subPath", json!("other")),
        ("/spec/volumes/0/secret/optional", json!(true)),
        ("/spec/volumes/0/secret/items", json!([])),
    ] {
        let mut changed = original.clone();
        if let Some(target) = changed.pointer_mut(path) {
            *target = value;
        }
        assert!(
            kubernetes::validate_pod(&changed, IMAGE, &request).is_err(),
            "{path}"
        );
    }
}

#[test]
fn secret_patch_is_identity_conditional_atomic_and_one_shot(
) -> Result<(), Box<dyn std::error::Error>> {
    let request = request();
    let original = kubernetes_fixture(&request, "Secret");
    let setup = render(Some(projection("Apply complete!".into())));
    let patch = kubernetes::patch(&original, &setup, &request).map_err(|_| "patch rejected")?;
    assert_eq!(
        patch[0],
        json!({"op":"test","path":"/metadata/uid","value":UID})
    );
    assert_eq!(
        patch[1],
        json!({"op":"test","path":"/metadata/resourceVersion","value":"125"})
    );
    assert_eq!(patch[2]["path"], "/data/.setup_info");
    assert_eq!(patch[2]["value"], STANDARD.encode(&setup));
    assert_eq!(
        patch[3],
        json!({"op":"add","path":"/immutable","value":true})
    );
    assert!(!serde_json::to_string(&patch)?.contains("private-jit"));
    let mut published = original.clone();
    published["data"][".setup_info"] = json!(STANDARD.encode(&setup));
    assert!(kubernetes::patch(&published, &setup, &request).is_err());
    published = original.clone();
    published["immutable"] = json!(true);
    assert!(kubernetes::patch(&published, &setup, &request).is_err());
    published = original;
    published["data"]["jit_config"] = json!(STANDARD.encode("different-jit"));
    assert!(kubernetes::patch(&published, &setup, &request).is_err());
    Ok(())
}

#[test]
fn transient_payload_files_are_private_and_removed() -> Result<(), Box<dyn std::error::Error>> {
    let directory = private_directory()?;
    let directory_path = directory.path().to_path_buf();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&directory_path)?.permissions().mode() & 0o777,
            0o700
        );
    }
    let file = protected_file(directory.path(), b"payload", false).map_err(|_| "write failed")?;
    let path = file.path().to_path_buf();
    assert_eq!(std::fs::read(&path)?, b"payload");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            file.as_file().metadata()?.permissions().mode() & 0o777,
            0o600
        );
        let readable =
            protected_file(directory.path(), b"sanitized", true).map_err(|_| "write failed")?;
        assert_eq!(
            readable.as_file().metadata()?.permissions().mode() & 0o777,
            0o444
        );
    }
    drop(file);
    assert!(!path.exists());
    drop(directory);
    assert!(!directory_path.exists());
    Ok(())
}

#[test]
fn secret_size_budget_uses_actual_jit_and_degrades_only_diagnostics(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut request = request();
    request.input.jit_config = "x".repeat(1024 * 1024 - 2);
    let secret = kubernetes_fixture(&request, "Secret");
    let original_data = secret["data"].clone();
    let setup = render(Some(projection("Apply complete!".into())));
    let patch = kubernetes::patch(&secret, &setup, &request).map_err(|_| "patch rejected")?;
    assert_eq!(patch[2]["value"], STANDARD.encode(b"[]"));
    assert_eq!(secret["data"], original_data);
    assert!(patch
        .as_array()
        .is_some_and(|operations| operations.iter().all(|operation| {
            operation["path"]
                .as_str()
                .is_some_and(|path| !path.contains("jit_config"))
        })));
    request.input.jit_config.push('x');
    let secret = kubernetes_fixture(&request, "Secret");
    assert!(kubernetes::patch(&secret, &setup, &request).is_err());
    Ok(())
}
