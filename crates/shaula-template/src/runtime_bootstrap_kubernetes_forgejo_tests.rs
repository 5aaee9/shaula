use super::*;
use shaula_core::{ports::forgejo::ForgejoBootstrapMaterial, secret::SecretString};
type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn pending_pod_gate_is_nonroot_single_use_and_has_no_extra_mounts() -> TestResult {
    let mut request = super::super::super::tests::request();
    let material = ForgejoBootstrapMaterial::new(
        "https://forgejo.test",
        "uuid",
        SecretString::new("one-shot"),
        vec!["linux:host".into()],
    )?;
    let mut pod = super::super::super::tests::kubernetes_fixture(&request, "Pod");
    let runner = &mut pod["spec"]["containers"][0];
    runner["image"] = json!("official-image");
    runner["command"] = json!(["/usr/bin/dumb-init", "--", "/bin/forgejo-runner"]);
    runner["args"] = command(&material);
    runner["env"] = json!([]);
    runner["securityContext"] = json!({"runAsNonRoot":true,"runAsUser":1000,"allowPrivilegeEscalation":false,"capabilities":{"drop":["ALL"]}});
    runner["volumeMounts"][0]["mountPath"] = json!("/data/.forgejo-token");
    runner["volumeMounts"][0]["subPath"] = json!("token");
    pod["spec"]["volumes"][0]["secret"]["items"] = json!([{"key":"token","path":"token"}]);
    request.input.jit_config.clear();
    request.input.generation.scale_set_id = None;
    request.input.forgejo = Some(material.identity());
    request.forgejo_bootstrap = Some(material);
    validate_pod(&pod, "official-image", &request).map_err(|_| "valid pod refused")?;
    let mut changed = pod.clone();
    changed["spec"]["containers"][0]["securityContext"]["allowPrivilegeEscalation"] = json!(true);
    assert!(validate_pod(&changed, "official-image", &request).is_err());
    let mut changed = pod.clone();
    changed["spec"]["volumes"]
        .as_array_mut()
        .ok_or("volumes")?
        .push(json!({"name":"host","hostPath":{"path":"/"}}));
    assert!(validate_pod(&changed, "official-image", &request).is_err());
    assert!(!pod.to_string().contains("one-shot"));

    let mut secret = super::super::super::tests::kubernetes_fixture(&request, "Secret");
    secret["data"] = json!({});
    let payload = patch(&secret, &request).map_err(|_| "empty gate refused")?;
    assert_eq!(payload[2]["value"]["token"], STANDARD.encode("one-shot"));
    assert_eq!(payload[3]["value"], true);
    secret["immutable"] = json!(true);
    assert!(patch(&secret, &request).is_err());
    Ok(())
}
