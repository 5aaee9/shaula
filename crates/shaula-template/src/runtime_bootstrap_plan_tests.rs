use super::*;
use serde_json::json;
#[path = "runtime_bootstrap_plan_fixture.rs"]
mod fixture;
use fixture::{fixture, TestResult};

#[test]
fn real_provider_defaults_and_computed_identity_fields_are_accepted() -> TestResult {
    for platform in ["docker", "kubernetes"] {
        let (manifest, request, plan) = fixture(platform)?;
        assert!(admit_plan(&manifest, &request, &plan).is_ok(), "{platform}");
    }
    Ok(())
}

#[test]
fn docker_refuses_early_start_unknown_controls_and_wrong_listener() -> TestResult {
    let (manifest, request, original) = fixture("docker")?;
    for (pointer, value) in [
        ("/resource_changes/0/change/after/start", json!(true)),
        ("/resource_changes/0/change/after/start", Value::Null),
        ("/resource_changes/0/change/after/must_run", json!(true)),
        (
            "/resource_changes/0/change/after/command",
            json!(["bootstrap-shim"]),
        ),
        (
            "/resource_changes/0/change/after/env",
            json!(["ACTIONS_RUNNER_INPUT_JITCONFIG=another-jit"]),
        ),
        (
            "/resource_changes/0/change/after/image",
            json!("unofficial-image"),
        ),
        (
            "/resource_changes/0/change/after_unknown",
            json!({"start":true}),
        ),
        (
            "/resource_changes/0/change/after_unknown",
            json!({"command":[false,true]}),
        ),
        (
            "/resource_changes/0/provider_name",
            json!("registry.terraform.io/custom/docker"),
        ),
    ] {
        let mut plan = original.clone();
        *plan.pointer_mut(pointer).ok_or("fixture pointer missing")? = value;
        assert!(
            matches!(
                admit_plan(&manifest, &request, &plan),
                Err(TemplateOutcomeError::PlanFailed { .. })
            ),
            "{pointer}"
        );
    }
    Ok(())
}

#[test]
fn docker_image_id_requires_the_official_resolved_data_source() -> TestResult {
    let (manifest, request, mut plan) = fixture("docker")?;
    plan["prior_state"]["values"]["root_module"]["resources"][0]["values"]["name"] =
        json!("custom/runner");
    assert!(admit_plan(&manifest, &request, &plan).is_err());
    let (manifest, request, mut plan) = fixture("docker")?;
    plan["planned_values"] = json!({"root_module":plan["prior_state"]["values"]["root_module"]});
    plan["prior_state"] = Value::Null;
    assert!(admit_plan(&manifest, &request, &plan).is_ok());
    Ok(())
}

#[test]
fn kubernetes_secret_gate_cannot_be_prepopulated_or_unknown() -> TestResult {
    let (manifest, request, original) = fixture("kubernetes")?;
    for (pointer, value) in [
        (
            "/resource_changes/1/change/after/data",
            json!({"jit_config":"frozen-jit",".setup_info":"[]"}),
        ),
        (
            "/resource_changes/1/change/after/binary_data",
            json!({".setup_info":"W10="}),
        ),
        ("/resource_changes/1/change/after/immutable", json!(true)),
        (
            "/resource_changes/1/change/after_unknown",
            json!({"data":true}),
        ),
        (
            "/resource_changes/1/change/after_unknown",
            json!({"data":{"jit_config":true}}),
        ),
        (
            "/resource_changes/1/change/after_unknown",
            json!({"binary_data":true}),
        ),
    ] {
        let mut plan = original.clone();
        *plan.pointer_mut(pointer).ok_or("fixture pointer missing")? = value;
        assert!(admit_plan(&manifest, &request, &plan).is_err(), "{pointer}");
    }
    Ok(())
}

#[test]
fn kubernetes_requires_pending_provider_and_a_known_nonoptional_volume_gate() -> TestResult {
    let (manifest, request, original) = fixture("kubernetes")?;
    for (pointer, value) in [
        (
            "/resource_changes/0/change/after/target_state",
            json!(["Running"]),
        ),
        (
            "/resource_changes/0/change/after/spec/0/init_container",
            json!([{"name":"shim"}]),
        ),
        (
            "/resource_changes/0/change/after/spec/0/container/0/image",
            json!("unofficial/runner"),
        ),
        (
            "/resource_changes/0/change/after/spec/0/container/0/command",
            json!(["bootstrap-shim"]),
        ),
        (
            "/resource_changes/0/change/after/spec/0/container/0/volume_mount/0/sub_path",
            Value::Null,
        ),
        (
            "/resource_changes/0/change/after/spec/0/volume/0/secret/0/optional",
            json!(true),
        ),
        (
            "/resource_changes/0/change/after/spec/0/volume/0/secret/0/secret_name",
            json!("another-generation"),
        ),
        (
            "/resource_changes/0/change/after/spec/0/volume/0/secret/0/items",
            json!([]),
        ),
        (
            "/resource_changes/0/change/after_unknown",
            json!({"spec":true}),
        ),
        (
            "/resource_changes/0/change/after_unknown",
            json!({"spec":[{"container":[true]}]}),
        ),
        (
            "/resource_changes/0/change/after_unknown",
            json!({"spec":[{"volume":[{"secret":[{"optional":true}]}]}]}),
        ),
    ] {
        let mut plan = original.clone();
        *plan.pointer_mut(pointer).ok_or("fixture pointer missing")? = value;
        assert!(admit_plan(&manifest, &request, &plan).is_err(), "{pointer}");
    }
    Ok(())
}

#[test]
fn resource_hooks_cannot_release_the_gate_during_apply() -> TestResult {
    let (manifest, request, mut plan) = fixture("docker")?;
    plan["configuration"]["root_module"]["module_calls"] = json!({"nested":{"module":{
        "resources":[{"provisioners":[{"type":"local-exec","expressions":{"command":{"constant_value":"docker start runner"}}}]}]
    }}});
    assert!(admit_plan(&manifest, &request, &plan).is_err());
    Ok(())
}

#[test]
fn legacy_contract_and_other_platform_plans_are_unchanged() -> TestResult {
    let (mut manifest, request, _) = fixture("docker")?;
    manifest.container_bootstrap_contract = None;
    assert!(admit_plan(&manifest, &request, &Value::Null).is_ok());
    Ok(())
}

#[test]
fn captured_local_provider_plan_oracles_match_when_explicitly_requested() -> TestResult {
    let Ok(directory) = std::env::var("SHAULA_BOOTSTRAP_PLAN_ORACLES") else {
        return Ok(());
    };
    for platform in ["docker", "kubernetes"] {
        let (manifest, mut request, _) = fixture(platform)?;
        let path = std::path::Path::new(&directory).join(format!("official-{platform}-plan.json"));
        let plan: Value = serde_json::from_slice(&std::fs::read(path)?)?;
        request.input = serde_json::from_value(plan["variables"]["shaula"]["value"].clone())?;
        assert!(
            admit_plan(&manifest, &request, &plan).is_ok(),
            "{platform} oracle"
        );
    }
    Ok(())
}
