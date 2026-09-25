use super::forgejo_pool_support::{Fixture, TestResult};
use sea_orm::ConnectionTrait;
use shaula_core::registry::{ChangeView, ControlPlaneStore, MutationFacts};

pub(super) async fn fixture() -> TestResult<Fixture> {
    let fixture = Fixture::new(0, 2).await?;
    fixture.store.store().connection().execute_unprepared(
        "INSERT INTO template_profiles
        (key,incarnation,desired_revision,active_revision,observed_revision,active_attestation_id,status,deletion_requested,created_at,updated_at)
        VALUES ('profile','tpl-inc',1,1,1,'attestation','Active',0,1,1);
        INSERT INTO github_auth_profiles
        (key,incarnation,desired_revision,active_revision,observed_revision,status,deletion_requested,created_at,updated_at)
        VALUES ('forgejo','auth-inc',1,1,1,'Active',0,1,1);"
    ).await?;
    Ok(fixture)
}

pub(super) fn facts(template: bool) -> MutationFacts {
    let (kind, key, incarnation) = if template {
        ("template_profile", "profile", "tpl-inc")
    } else {
        ("github_auth_profile", "forgejo", "auth-inc")
    };
    MutationFacts {
        idempotency_operation: "v1:PUT",
        authentication: Default::default(),
        resource_kind: kind,
        resource_key: key.into(),
        incarnation: incarnation.into(),
        revision: 1,
        spec_json: String::new(),
        template: None,
        template_pool: vec![],
        template_pool_ref: None,
        auth_desired: None,
        inputs_digest: String::new(),
        actor: "test".into(),
        now: 10,
        change: ChangeView {
            id: format!("retire-{key}"),
            resource_kind: kind.into(),
            resource_key: key.into(),
            revision: 1,
            kind: "Retire".into(),
            state: "Blocked".into(),
            reason: Some("ResourceInUse".into()),
        },
        outbox_topic: "profile.retire".into(),
        outbox_payload: "{}".into(),
        idempotency: None,
    }
}

pub(super) async fn retire(fixture: &Fixture) -> TestResult {
    for template in [true, false] {
        assert!(fixture
            .store
            .commit_profile_retirement(facts(template))
            .await?
            .is_ok());
    }
    Ok(())
}

pub(super) async fn status(fixture: &Fixture, expected: &str) -> TestResult {
    for head in [
        fixture.store.template_profile_get("profile").await?,
        fixture.store.auth_profile_get("forgejo").await?,
    ] {
        let head = head.ok_or("missing profile")?;
        assert_eq!(head.status, expected);
        assert_eq!(
            head.active_revision,
            if expected == "Retired" { None } else { Some(1) }
        );
    }
    Ok(())
}

pub(super) fn pool(fixture: &Fixture) -> TestResult<MutationFacts> {
    let mut facts = facts(true);
    facts.resource_kind = "template_pool";
    facts.resource_key = "pool".into();
    facts.incarnation = "pool-inc".into();
    facts.spec_json = r#"{"members":[{"key":"one","template_profile_ref":"profile","weight":1}],"failure_policy":"backpressure"}"#.into();
    facts.template_pool = vec![shaula_core::template_pool::ResolvedTemplatePoolMember {
        key: "one".into(),
        template_profile_key: "profile".into(),
        template_revision: 1,
        template_artifact_digest: format!("sha256:{}", "a".repeat(64)),
        template_attestation_id: "attestation".into(),
        template_inputs: fixture.spec.template_inputs.clone(),
        inputs_digest: "inputs".into(),
        weight: 1,
        max_runners: None,
    }];
    facts.change.id = "pool-put".into();
    facts.change.kind = "Create".into();
    Ok(facts)
}
