//! Commit implementations, split to keep files under 400 lines.

use shaula_core::error::CoreResult;
use shaula_core::registry::{AttestationRecord, AuthRevisionRow, MutationError, MutationFacts};

use super::core_err;
use super::SqliteControlPlane;

impl SqliteControlPlane {
    /// The replay pre-check lookup (R6-07): derives the FULL path
    /// identity (Profile, Revision, key) and returns the persisted
    /// record's canonical members.
    pub(crate) async fn attestation_lookup(
        &self,
        profile_key: &str,
        revision: i64,
        attestation_key: &str,
    ) -> CoreResult<Option<shaula_core::registry::AttestationReplayRow>> {
        let id =
            shaula_core::registry::attestation_record_id(profile_key, revision, attestation_key);
        Ok(self
            .store
            .template_attestation_get_by_id(&id)
            .await
            .map_err(core_err)?
            .map(|row| shaula_core::registry::AttestationReplayRow {
                profile_key: row.profile_key,
                revision: row.revision,
                subject_json: row.subject_json,
                result: row.result,
                evidence_digest: row.evidence_digest,
                suite_name: row.suite_name,
                suite_version: row.suite_version,
                completed_at: row.completed_at,
                subject_verified: row.subject_verified,
            }))
    }

    pub(crate) async fn commit_template_revision_impl(
        &self,
        facts: MutationFacts,
        extra: (String, String, String, String),
        source_key: Option<String>,
    ) -> CoreResult<Result<(), MutationError>> {
        let (engine_ref, bindings_json, bindings_digest, fleet_input_policy_json) = extra;
        let tx = self.store.begin().await.map_err(core_err)?;
        if let Err(e) = self
            .store
            .template_commit_revision(
                &tx,
                shaula_core::template::TemplateRevisionInsert {
                    key: facts.resource_key.clone(),
                    incarnation: facts.incarnation.clone(),
                    revision: facts.revision,
                    artifact_digest: facts.inputs_digest.clone(),
                    engine_ref,
                    source_key,
                    bindings_json: Some(bindings_json),
                    bindings_digest: Some(bindings_digest),
                    fleet_input_policy_json: Some(fleet_input_policy_json),
                },
                facts.now,
            )
            .await
        {
            // R9-02: a lost fence race (a concurrent PUT advanced the
            // desired head between admission and commit) is a DOMAIN
            // rejection — surface the current head as a precondition
            // failure, never overwrite and never 500.
            if matches!(e, crate::store::StoreError::Conflict { .. }) {
                let _ = tx.rollback().await;
                let current = self
                    .store
                    .template_profile_get(&facts.resource_key)
                    .await
                    .map_err(core_err)?;
                return Ok(Err(match current {
                    Some(head) => MutationError::PreconditionFailed {
                        current: (head.incarnation, head.desired_revision),
                    },
                    None => MutationError::NotFound,
                }));
            }
            return Err(core_err(e));
        }
        self.store
            .profile_change_insert(
                &tx,
                shaula_core::registry::ProfileChangeInsert {
                    id: facts.change.id.clone(),
                    resource_kind: "template_profile".into(),
                    profile_key: facts.resource_key.clone(),
                    revision: Some(facts.revision),
                    kind: facts.change.kind.clone(),
                    now: facts.now,
                },
            )
            .await
            .map_err(core_err)?;
        self.store
            .audit_append(
                &tx,
                shaula_core::registry::AuditAppend {
                    resource_kind: "template_profile".into(),
                    action: "put".into(),
                    actor: facts.actor.clone(),
                    resource_key: facts.resource_key.clone(),
                    revision: Some(facts.revision),
                    outcome: "accepted".into(),
                    detail_json: None,
                    now: facts.now,
                },
            )
            .await
            .map_err(core_err)?;
        self.store
            .outbox_enqueue(
                &tx,
                "template_profile",
                &facts.outbox_topic,
                &facts.outbox_payload,
                facts.now,
            )
            .await
            .map_err(core_err)?;
        if let Some((idem_key, request_hash, status, body)) = &facts.idempotency {
            self.store
                .idempotency_store(
                    &tx,
                    shaula_core::registry::IdempotencyInsert {
                        id: format!("idem-{}", facts.change.id),
                        resource_kind: facts.resource_kind.to_string(),
                        resource_key: facts.resource_key.clone(),
                        idempotency_key: idem_key.clone(),
                        request_hash: request_hash.clone(),
                        response_status: *status,
                        response_body: Some(body.clone()),
                        now: facts.now,
                    },
                )
                .await
                .map_err(core_err)?;
        }
        tx.commit()
            .await
            .map_err(|e| core_err(crate::store::StoreError::from(e)))?;
        Ok(Ok(()))
    }

    pub(crate) async fn commit_auth_revision_impl(
        &self,
        facts: MutationFacts,
        credential: AuthRevisionRow,
        secret_bytes: &[u8],
    ) -> CoreResult<Result<(), MutationError>> {
        let tx = self.store.begin().await.map_err(core_err)?;
        if let Err(error) = self
            .store
            .auth_commit_revision(
                &tx,
                shaula_core::auth::AuthRevisionInsert {
                    key: facts.resource_key.clone(),
                    incarnation: facts.incarnation.clone(),
                    revision: facts.revision,
                    kind: credential.kind.clone(),
                    app_id: credential.app_id.clone(),
                    schema_version: credential.schema_version,
                    policy_json: credential.policy_json.clone(),
                },
                secret_bytes,
                facts.now,
            )
            .await
        {
            let _ = tx.rollback().await;
            if matches!(error, crate::store::StoreError::Conflict { .. }) {
                let current = self
                    .store
                    .auth_profile_get(&facts.resource_key)
                    .await
                    .map_err(core_err)?;
                return Ok(Err(match current {
                    Some(head) => MutationError::PreconditionFailed {
                        current: (head.incarnation, head.desired_revision),
                    },
                    None => MutationError::NotFound,
                }));
            }
            return Err(core_err(error));
        }
        self.store
            .profile_change_insert(
                &tx,
                shaula_core::registry::ProfileChangeInsert {
                    id: facts.change.id.clone(),
                    resource_kind: "github_auth_profile".into(),
                    profile_key: facts.resource_key.clone(),
                    revision: Some(facts.revision),
                    kind: facts.change.kind.clone(),
                    now: facts.now,
                },
            )
            .await
            .map_err(core_err)?;
        self.store
            .audit_append(
                &tx,
                shaula_core::registry::AuditAppend {
                    resource_kind: "github_auth_profile".into(),
                    action: "put".into(),
                    actor: facts.actor.clone(),
                    resource_key: facts.resource_key.clone(),
                    revision: Some(facts.revision),
                    outcome: "accepted".into(),
                    detail_json: None,
                    now: facts.now,
                },
            )
            .await
            .map_err(core_err)?;
        self.store
            .outbox_enqueue(
                &tx,
                "github_auth_profile",
                &facts.outbox_topic,
                &facts.outbox_payload,
                facts.now,
            )
            .await
            .map_err(core_err)?;
        if let Some((idem_key, request_hash, status, body)) = &facts.idempotency {
            self.store
                .idempotency_store(
                    &tx,
                    shaula_core::registry::IdempotencyInsert {
                        id: format!("idem-{}", facts.change.id),
                        resource_kind: facts.resource_kind.to_string(),
                        resource_key: facts.resource_key.clone(),
                        idempotency_key: idem_key.clone(),
                        request_hash: request_hash.clone(),
                        response_status: *status,
                        response_body: Some(body.clone()),
                        now: facts.now,
                    },
                )
                .await
                .map_err(core_err)?;
        }
        tx.commit()
            .await
            .map_err(|e| core_err(crate::store::StoreError::from(e)))?;
        Ok(Ok(()))
    }

    /// Stores immutable conformance evidence, its audit and outbox together.
    /// Evidence never changes Template activation (spec 0017).
    pub(crate) async fn commit_attestation_impl(
        &self,
        record: AttestationRecord,
        actor: String,
        now: i64,
    ) -> CoreResult<
        Result<shaula_core::registry::AttestationCommit, shaula_core::registry::MutationError>,
    > {
        use shaula_core::registry::{AttestationCommit, MutationError};
        let tx = self.store.begin().await.map_err(core_err)?;

        // The FULL path identity (Profile, Revision, attestation key) is
        // the record's STABLE identity: an exact replay returns the
        // original record; the same identity with a different canonical
        // body conflicts.
        if let Some(existing) = self
            .store
            .template_attestation_get(&tx, &record.id)
            .await
            .map_err(core_err)?
        {
            let identical = existing.profile_key == record.profile_key
                && existing.revision == record.revision
                && existing.subject_json == record.subject_json
                && existing.result == record.result
                && existing.evidence_digest == record.evidence_digest
                && existing.suite_name.as_deref() == Some(record.suite.0.as_str())
                && existing.suite_version.as_deref() == Some(record.suite.1.as_str())
                && existing.completed_at == record.completed_at;
            if !identical {
                return Ok(Err(MutationError::IdempotencyConflict));
            }
            // Nothing was written; the original record and its frozen
            // activation (if any) stand untouched.
            return Ok(Ok(AttestationCommit::Replayed));
        }

        self.store
            .template_attestation_insert(
                &tx,
                shaula_core::template::AttestationInsert {
                    id: record.id.clone(),
                    key: record.profile_key.clone(),
                    revision: record.revision,
                    subject_json: record.subject_json.clone(),
                    subject_digest: record.subject_digest.clone(),
                    result: record.result.clone(),
                    evidence_digest: record.evidence_digest.clone(),
                    suite_name: Some(record.suite.0.clone()),
                    suite_version: Some(record.suite.1.clone()),
                    completed_at: record.completed_at,
                    subject_verified: record.subject_verified,
                },
            )
            .await
            .map_err(core_err)?;
        // The audit fact carries actor, Profile Revision, attestation
        // key, subject digest, suite version and the sanitized result —
        // never raw output, bindings or credentials (spec 0005 §5). The
        // verification verdict is part of the durable audit trail (R6-08).
        let detail = serde_json::json!({
            "attestation_key": record.attestation_key,
            "subject_digest": record.subject_digest,
            "suite_version": record.suite.1,
            "result": record.result,
            "subject_verified": record.subject_verified,
        })
        .to_string();
        let audit_outcome = if record.subject_verified {
            record.result.clone()
        } else {
            "mismatched".to_string()
        };
        self.store
            .audit_append(
                &tx,
                shaula_core::registry::AuditAppend {
                    resource_kind: "template_profile".into(),
                    action: "attest".into(),
                    actor: actor.clone(),
                    resource_key: record.profile_key.clone(),
                    revision: Some(record.revision),
                    outcome: audit_outcome,
                    detail_json: Some(detail),
                    now,
                },
            )
            .await
            .map_err(core_err)?;
        self.store
            .outbox_enqueue(
                &tx,
                "template_profile",
                "profile.attest",
                &format!(
                    "{{\"key\":\"{}\",\"revision\":{}}}",
                    record.profile_key, record.revision
                ),
                now,
            )
            .await
            .map_err(core_err)?;

        tx.commit()
            .await
            .map_err(|e| core_err(crate::store::StoreError::from(e)))?;
        Ok(Ok(AttestationCommit::Created))
    }
}
