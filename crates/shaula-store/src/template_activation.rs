//! Fenced static-validation completion and automatic Template activation.

use sea_orm::{sea_query::Expr, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};

use crate::entities::template::{profile_changes, template_profile_revisions, template_profiles};
use crate::{Store, StoreResult};

pub(crate) const VALIDATOR_VERSION: &str = "static-validation-v1";

/// Material read and validated before entering the short database transaction.
pub(crate) struct ValidatedTemplate {
    pub platform: String,
    pub bindings_contract: String,
    pub manifest_json: String,
    pub lock_digest: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValidationCommit {
    Activated,
    Rejected,
    Superseded,
}

/// Legacy reasons may contain parser input. Only bounded codes leave the store.
pub(crate) fn public_validation_reason(reason: Option<String>) -> Option<String> {
    reason.map(|reason| match reason.as_str() {
        "ArtifactNotPublished"
        | "ArtifactShapeInvalid"
        | "ManifestInvalid"
        | "DependencyLockInvalid" => reason,
        _ => "ValidationFailed".to_string(),
    })
}

impl Store {
    /// A stale scan is a no-op. All accepted/rejected facts commit together;
    /// a storage failure rolls the transaction back, allowing the next scan.
    pub(crate) async fn template_validation_commit(
        &self,
        expected_profile: &template_profiles::Model,
        expected_candidate: &template_profile_revisions::Model,
        validation: Result<ValidatedTemplate, &'static str>,
        now: i64,
    ) -> StoreResult<ValidationCommit> {
        let tx = self.begin().await?;
        let Some(profile) = template_profiles::Entity::find_by_id(&expected_profile.key)
            .one(&tx)
            .await?
        else {
            return Ok(ValidationCommit::Superseded);
        };
        if profile.incarnation != expected_profile.incarnation
            || profile.desired_revision != expected_candidate.revision
            || profile.deletion_requested
            || matches!(profile.status.as_str(), "Retiring" | "Retired")
            || profile.active_revision == Some(expected_candidate.revision)
        {
            return Ok(ValidationCommit::Superseded);
        }
        let Some(candidate) = template_profile_revisions::Entity::find_by_id(expected_candidate.id)
            .one(&tx)
            .await?
        else {
            return Ok(ValidationCommit::Superseded);
        };
        if candidate.profile_key != profile.key
            || candidate.revision != expected_candidate.revision
            || candidate.artifact_digest != expected_candidate.artifact_digest
            || candidate.engine_ref != expected_candidate.engine_ref
            || candidate.bindings_digest != expected_candidate.bindings_digest
            || !matches!(candidate.state.as_str(), "Validating" | "Ready")
        {
            return Ok(ValidationCommit::Superseded);
        }

        let activation_id = format!(
            "{VALIDATOR_VERSION}:{}",
            shaula_core::auth::request_hash_parts(&[
                VALIDATOR_VERSION.as_bytes(),
                profile.key.as_bytes(),
                profile.incarnation.as_bytes(),
                &candidate.revision.to_be_bytes(),
                candidate.artifact_digest.as_bytes(),
            ])
        );
        let (outcome, status, reason) = match &validation {
            Ok(_) => (ValidationCommit::Activated, "Active", None),
            Err(reason) => (ValidationCommit::Rejected, "Rejected", Some(*reason)),
        };
        let detail = serde_json::json!({
            "activation_id": validation.as_ref().ok().map(|_| activation_id.as_str()),
            "method": "static_validation",
            "validator_version": VALIDATOR_VERSION,
            "incarnation": profile.incarnation,
            "artifact_digest": candidate.artifact_digest,
            "engine_ref": candidate.engine_ref,
            "bindings_digest": candidate.bindings_digest,
            "lock_digest": validation.as_ref().ok().map(|v| v.lock_digest.as_str()),
            "reason": reason,
        });
        let mut updated_candidate: template_profile_revisions::ActiveModel = candidate.into();
        updated_candidate.state = Set(status.to_string());
        updated_candidate.reason = Set(reason.map(str::to_string));
        if let Ok(validated) = validation {
            updated_candidate.platform = Set(Some(validated.platform));
            updated_candidate.bindings_contract = Set(Some(validated.bindings_contract));
            updated_candidate.manifest_json = Set(Some(validated.manifest_json));
            updated_candidate.lock_digest = Set(Some(validated.lock_digest));
        }
        template_profile_revisions::Entity::update(updated_candidate)
            .exec(&tx)
            .await?;

        let mut updated_profile: template_profiles::ActiveModel = profile.clone().into();
        updated_profile.status = Set(status.to_string());
        updated_profile.observed_revision = Set(Some(profile.desired_revision));
        updated_profile.updated_at = Set(now);
        if outcome == ValidationCommit::Activated {
            updated_profile.active_revision = Set(Some(profile.desired_revision));
            // Historical name retained for opaque activation provenance, not
            // a claim that a conformance attestation exists for this ID.
            updated_profile.active_attestation_id = Set(Some(activation_id));
        }
        template_profiles::Entity::update(updated_profile)
            .exec(&tx)
            .await?;
        profile_changes::Entity::update_many()
            .col_expr(
                profile_changes::Column::State,
                Expr::value(if outcome == ValidationCommit::Activated {
                    "Converged"
                } else {
                    "Rejected"
                }),
            )
            .col_expr(profile_changes::Column::Reason, Expr::value(reason))
            .col_expr(
                profile_changes::Column::NextRetryAt,
                Expr::value(None::<i64>),
            )
            .col_expr(profile_changes::Column::UpdatedAt, Expr::value(now))
            .filter(profile_changes::Column::ResourceKind.eq("template_profile"))
            .filter(profile_changes::Column::ProfileKey.eq(&profile.key))
            .filter(profile_changes::Column::Revision.eq(profile.desired_revision))
            .filter(profile_changes::Column::Kind.eq("Publish"))
            .filter(profile_changes::Column::State.is_in(["Pending", "Retrying"]))
            .exec(&tx)
            .await?;
        self.audit_append(
            &tx,
            shaula_core::registry::AuditAppend {
                resource_kind: "template_profile".into(),
                action: if outcome == ValidationCommit::Activated {
                    "activate"
                } else {
                    "validate"
                }
                .into(),
                actor: "system:template-validator".into(),
                resource_key: profile.key.clone(),
                revision: Some(profile.desired_revision),
                outcome: status.into(),
                detail_json: Some(detail.to_string()),
                now,
            },
        )
        .await?;
        self.outbox_enqueue(
            &tx,
            "template_profile",
            if outcome == ValidationCommit::Activated {
                "profile.activate"
            } else {
                "profile.reject"
            },
            &serde_json::json!({"key": profile.key, "revision": profile.desired_revision})
                .to_string(),
            now,
        )
        .await?;
        tx.commit().await?;
        Ok(outcome)
    }
}
