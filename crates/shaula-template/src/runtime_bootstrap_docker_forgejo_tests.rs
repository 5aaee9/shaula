use super::*;
use shaula_core::{ports::forgejo::ForgejoBootstrapMaterial, secret::SecretString};
type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn official_image_volume_and_entrypoint_are_required() -> TestResult {
    let mut request = super::super::super::tests::request();
    let material = ForgejoBootstrapMaterial::new(
        "https://forgejo.test",
        "uuid",
        SecretString::new("one-shot"),
        vec!["linux:host".into()],
    )?;
    let mut info = super::super::super::tests::docker_fixture(&request);
    info["Config"]["Cmd"] = command(&material);
    info["Config"]["Entrypoint"] = json!(["/usr/bin/dumb-init"]);
    info["Config"]["WorkingDir"] = json!("/data");
    info["Config"]["Volumes"] = json!({"/data":{}});
    info["Config"]["Env"] = json!(["HOME=/data"]);
    info["Mounts"] = json!([{"Type":"volume","Destination":"/data","Driver":"local","RW":true,"Name":"a".repeat(64)}]);
    request.input.jit_config.clear();
    request.input.generation.scale_set_id = None;
    request.input.forgejo = Some(material.identity());
    request.forgejo_bootstrap = Some(material);
    validate(&info, "container-id", "image-id", &request)
        .map_err(|_| "valid official fixture refused")?;
    for (pointer, value) in [
        ("/Config/Entrypoint", json!([])),
        ("/Mounts/0/Type", json!("bind")),
        ("/Mounts/0/Name", json!("external-volume")),
        ("/Config/WorkingDir", json!("/home/runner")),
        ("/HostConfig/Binds", json!(["/host:/data"])),
    ] {
        let mut changed = info.clone();
        if pointer == "/HostConfig/Binds" {
            changed["HostConfig"]["Binds"] = value;
        } else {
            *changed
                .pointer_mut(pointer)
                .ok_or("fixture pointer missing")? = value;
        }
        assert!(
            validate(&changed, "container-id", "image-id", &request).is_err(),
            "{pointer}"
        );
    }
    let rendered = info.to_string();
    assert!(!rendered.contains("one-shot"));
    assert!(rendered.contains("file:///data/.forgejo-token"));
    Ok(())
}
