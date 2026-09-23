use super::*;

fn base_spec() -> FleetSpec {
    FleetSpec {
        kind: FleetProviderKind::Github,
        github: FleetGithubSection {
            target: GitHubTarget::organization("example-org").unwrap(),
            auth_profile_ref: "production-app".into(),
            scale_set_name: "shaula-linux-x64".into(),
            runner_group: "Default".into(),
            labels: vec!["shaula-linux-x64".into()],
        },
        forgejo: None,
        capacity: CapacityPolicyDto {
            min_runners: 0,
            max_runners: 20,
        },
        template_profile_ref: TemplateProfileRefDto::from("kubernetes-linux-x64".to_string()),
        template_pool: None,
        template_pool_ref: None,
        template_inputs: serde_json::Map::new(),
    }
}

#[test]
fn valid_spec_passes() {
    assert!(validate_fleet_spec(&base_spec()).is_ok());
}

#[test]
fn capacity_inversion_rejected() {
    let mut spec = base_spec();
    spec.capacity = CapacityPolicyDto {
        min_runners: 10,
        max_runners: 5,
    };
    assert_eq!(
        validate_fleet_spec(&spec).unwrap_err().code,
        ReasonCode::SpecInvalid
    );
}

#[test]
fn unknown_fields_rejected() {
    let mut raw = serde_json::to_value(base_spec()).unwrap();
    raw["evil_field"] = serde_json::json!({"namespace": "should-fail"});
    assert!(serde_json::from_value::<FleetSpec>(raw).is_err());
}

#[test]
fn bare_key_deserializes_and_legacy_pin_normalizes() {
    let bare: TemplateProfileRefDto = serde_json::from_str("\"tpl\"").unwrap();
    assert_eq!(bare.key(), "tpl");
    let legacy: TemplateProfileRefDto =
        serde_json::from_str(r#"{"key":"tpl","revision":4}"#).unwrap();
    assert_eq!(legacy.key(), "tpl");
    assert_eq!(serde_json::to_string(&legacy).unwrap(), "\"tpl\"");
}

#[test]
fn invalid_fleet_key_rejected() {
    assert!(FleetKey::new("has space").is_err());
    assert!(FleetKey::new("linux-x64.1_a").is_ok());
    assert!(FleetKey::new("\nlinux-x64").is_err());
    assert!(FleetKey::new("linux-x64 ").is_err());
}

fn pool() -> TemplatePoolSpec {
    TemplatePoolSpec {
        members: vec![TemplatePoolMember {
            key: "a".into(),
            template_profile_ref: "tpl".to_string().into(),
            weight: 1,
            template_inputs: serde_json::Map::new(),
            max_runners: None,
        }],
        failure_policy: PoolFailurePolicy::Backpressure,
    }
}

#[test]
fn pool_requires_members_and_exclusive_template_reference() {
    let mut spec = base_spec();
    spec.template_profile_ref = TemplateProfileRefDto::default();
    spec.template_pool = Some(TemplatePoolSpec {
        members: Vec::new(),
        ..pool()
    });
    assert_eq!(
        validate_fleet_spec(&spec).unwrap_err().code,
        ReasonCode::SpecInvalid
    );
    spec.template_pool = Some(pool());
    assert!(validate_fleet_spec(&spec).is_ok());
    spec.template_profile_ref = "legacy".to_string().into();
    assert_eq!(
        validate_fleet_spec(&spec).unwrap_err().code,
        ReasonCode::SpecInvalid
    );
}

#[test]
fn forgejo_admits_each_exclusive_template_source_and_omits_github_on_read() {
    let mut spec = FleetSpec {
        kind: FleetProviderKind::Forgejo,
        github: FleetGithubSection::default(),
        forgejo: Some(FleetForgejoSection {
            instance_url: "https://forgejo.example.test".into(),
            scope: crate::forgejo::ForgejoScope::Instance,
            auth_profile_ref: "forgejo".into(),
            runner_name_prefix: "shaula-test-".into(),
            labels: vec!["linux:host".into()],
        }),
        ..base_spec()
    };
    assert!(validate_fleet_spec(&spec).is_ok());
    let json = serde_json::to_value(&spec).unwrap();
    assert!(json.get("github").is_none());
    assert_eq!(json["kind"], "forgejo");
    spec.template_profile_ref = TemplateProfileRefDto::default();
    spec.template_pool_ref = Some("shared".into());
    assert!(validate_fleet_spec(&spec).is_ok());
    spec.template_pool_ref = None;
    spec.template_pool = Some(pool());
    assert!(validate_fleet_spec(&spec).is_ok());
    spec.template_pool_ref = Some("shared".into());
    assert_eq!(
        validate_fleet_spec(&spec).unwrap_err().code,
        ReasonCode::SpecInvalid
    );
}
