use super::{admit_plan, fixture, ProfileManifest, TemplateCreateRequest, TestResult, Value};
use serde_json::json;
use shaula_core::{ports::forgejo::ForgejoBootstrapMaterial, secret::SecretString};

fn forgejo_fixture(
    platform: &str,
) -> Result<(ProfileManifest, TemplateCreateRequest, Value), Box<dyn std::error::Error>> {
    let (manifest, mut request, mut plan) = fixture(platform)?;
    let material = ForgejoBootstrapMaterial::new(
        "https://forgejo.test",
        "runner-uuid",
        SecretString::new("protected-token-marker"),
        vec!["linux:host".into()],
    )?;
    request
        .input
        .bindings
        .insert("runner_backend".into(), json!("forgejo"));
    request.input.generation.scale_set_id = None;
    request.input.jit_config.clear();
    request.input.forgejo = Some(material.identity());
    let image =
        manifest.selected_runner_image(&request.input.bindings, &request.input.parameters)?;
    if platform == "docker" {
        let runner = &mut plan["resource_changes"][0]["change"];
        let mut command = vec!["/bin/forgejo-runner".to_string()];
        command.extend(material.one_job_args());
        runner["after"]["command"] = json!(command);
        runner["after"]["entrypoint"] = json!(["/usr/bin/dumb-init"]);
        runner["after"]["working_dir"] = json!("/data");
        runner["after"]["remove_volumes"] = json!(true);
        runner["after"]["env"] = json!(["SHAULA_RUNNER_BACKEND=forgejo"]);
        runner["after_unknown"]["entrypoint"] = json!(false);
        plan["prior_state"]["values"]["root_module"]["resources"][0]["values"]["name"] =
            json!(image);
    } else {
        let pod = &mut plan["resource_changes"][0]["change"]["after"]["spec"][0];
        let runner = &mut pod["container"][0];
        runner["image"] = json!(image);
        runner["command"] = json!(["/usr/bin/dumb-init", "--", "/bin/forgejo-runner"]);
        runner["args"] = json!(material.one_job_args());
        runner["env"] = json!([]);
        runner["security_context"] = json!([{"run_as_non_root":true, "run_as_user":1000,
            "allow_privilege_escalation":false, "capabilities":[{"drop":["ALL"],"add":[]}]}]);
        runner["volume_mount"][0]["mount_path"] = json!("/data/.forgejo-token");
        runner["volume_mount"][0]["sub_path"] = json!("token");
        pod["volume"][0]["secret"][0]["items"] = json!([{"key":"token","path":"token"}]);
        plan["resource_changes"][1]["change"]["after"]["data"] =
            json!({"runner_backend":"forgejo"});
    }
    request.forgejo_bootstrap = Some(material);
    Ok((manifest, request, plan))
}

#[test]
fn same_manifest_admits_only_the_published_backends_plan_and_identity() -> TestResult {
    for platform in ["docker", "kubernetes"] {
        let (manifest, mut request, plan) = forgejo_fixture(platform)?;
        assert!(admit_plan(&manifest, &request, &plan).is_ok(), "{platform}");
        assert!(!request
            .input
            .to_tfvars()?
            .contains("protected-token-marker"));
        assert!(!plan.to_string().contains("protected-token-marker"));
        let (_, _, github_plan) = fixture(platform)?;
        assert!(admit_plan(&manifest, &request, &github_plan).is_err());
        request
            .input
            .bindings
            .insert("runner_backend".into(), json!("github"));
        assert!(admit_plan(&manifest, &request, &plan).is_err());
    }
    Ok(())
}

#[test]
fn forgejo_plan_keeps_token_gate_and_volume_cleanup() -> TestResult {
    let (manifest, request, original) = forgejo_fixture("docker")?;
    for (pointer, value) in [
        (
            "/resource_changes/0/change/after/remove_volumes",
            json!(false),
        ),
        (
            "/resource_changes/0/change/after/env",
            json!(["TOKEN=unsafe"]),
        ),
        ("/resource_changes/0/change/after_unknown/env", json!(true)),
    ] {
        let mut plan = original.clone();
        *plan.pointer_mut(pointer).ok_or("fixture pointer missing")? = value;
        assert!(admit_plan(&manifest, &request, &plan).is_err(), "{pointer}");
    }
    let (manifest, request, original) = forgejo_fixture("kubernetes")?;
    for (pointer, value) in [
        (
            "/resource_changes/1/change/after/data",
            json!({"token":"unsafe"}),
        ),
        (
            "/resource_changes/0/change/after/spec/0/container/0/security_context/0/run_as_user",
            json!(0),
        ),
        (
            "/resource_changes/0/change/after/spec/0/container/0/args",
            json!(["daemon"]),
        ),
        (
            "/resource_changes/0/change/after/spec/0/volume/0/secret/0/optional",
            json!(true),
        ),
    ] {
        let mut plan = original.clone();
        *plan.pointer_mut(pointer).ok_or("fixture pointer missing")? = value;
        assert!(admit_plan(&manifest, &request, &plan).is_err(), "{pointer}");
    }
    Ok(())
}

#[test]
fn captured_forgejo_provider_plans_match_when_explicitly_requested() -> TestResult {
    let Ok(directory) = std::env::var("SHAULA_BOOTSTRAP_PLAN_ORACLES") else {
        return Ok(());
    };
    for platform in ["docker", "kubernetes"] {
        let (manifest, mut request, _) = forgejo_fixture(platform)?;
        let plan: Value = serde_json::from_slice(&std::fs::read(
            std::path::Path::new(&directory).join(format!("forgejo-{platform}-plan.json")),
        )?)?;
        request.input = serde_json::from_value(plan["variables"]["shaula"]["value"].clone())?;
        let identity = request.input.forgejo.as_ref().ok_or("identity missing")?;
        request.forgejo_bootstrap = Some(ForgejoBootstrapMaterial::new(
            &identity.instance_url,
            &identity.uuid,
            SecretString::new("test-token"),
            identity.labels.clone(),
        )?);
        admit_plan(&manifest, &request, &plan)
            .map_err(|_| format!("{platform} Forgejo oracle rejected"))?;
    }
    Ok(())
}
