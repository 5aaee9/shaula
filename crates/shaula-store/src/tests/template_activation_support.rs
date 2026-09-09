use crate::entities::template::{template_profile_revisions, template_profiles};
use crate::template_activation::ValidatedTemplate;
use crate::Store;

pub(super) type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

pub(super) struct Fixture {
    pub temp: tempfile::TempDir,
    pub store: Store,
}

impl Fixture {
    pub async fn new() -> TestResult<Self> {
        let temp = tempfile::tempdir()?;
        let store = Store::open(&temp.path().join("activation.db")).await?;
        store.migrate().await?;
        let fixture = Self { temp, store };
        fixture.publish(1).await?;
        Ok(fixture)
    }

    pub async fn publish(&self, revision: i64) -> TestResult {
        let tx = self.store.begin().await?;
        self.store
            .template_commit_revision(
                &tx,
                shaula_core::template::TemplateRevisionInsert {
                    key: "tpl".into(),
                    incarnation: "inc-1".into(),
                    revision,
                    artifact_digest: format!("sha256:{}", "a".repeat(64)),
                    engine_ref: "terraform".into(),
                    source_key: None,
                    bindings_json: Some("{\"secret\":\"protected-value\"}".into()),
                    bindings_digest: Some("bd1_protected".into()),
                    fleet_input_policy_json: Some("{}".into()),
                },
                revision,
            )
            .await?;
        self.store
            .profile_change_insert(
                &tx,
                shaula_core::registry::ProfileChangeInsert {
                    id: format!("publish-{revision}"),
                    resource_kind: "template_profile".into(),
                    profile_key: "tpl".into(),
                    revision: Some(revision),
                    kind: "Publish".into(),
                    now: revision,
                },
            )
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn snapshot(
        &self,
    ) -> TestResult<(template_profiles::Model, template_profile_revisions::Model)> {
        let profile = self
            .store
            .template_profile_get("tpl")
            .await?
            .ok_or("profile missing")?;
        let revision = self
            .store
            .template_revision_get("tpl", profile.desired_revision)
            .await?
            .ok_or("revision missing")?;
        Ok((profile, revision))
    }
}

pub(super) fn validated() -> ValidatedTemplate {
    ValidatedTemplate {
        platform: "docker".into(),
        bindings_contract: "shaula.bindings.docker/v1".into(),
        manifest_json: "validated test material".into(),
        lock_digest: "sha256:lock".into(),
    }
}
