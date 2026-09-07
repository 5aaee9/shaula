//! Attestation PUT application logic, split from service_profile_registry
//! to stay within the 400-line limit (AGENTS.md).

use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::registry::{Actor, AttestationPut, AttestationRecord, MutationError, Scope};

use super::verify_attestation_subject;
use super::{request_hash, unprocessable, ControlPlane};

impl ControlPlane {
    /// Attestation PUT (spec 0005 §5, R6-06/07/08): replay pre-check by
    /// FULL path identity FIRST, then subject verification against the
    /// current authority, then a single-transaction durable commit.
    pub(crate) async fn attestation_put_impl(
        &self,
        actor: &Actor,
        key: &str,
        revision: i64,
        payload: AttestationPut,
    ) -> CoreResult<Result<String, MutationError>> {
        if !actor.has(Scope::TemplateAttest) {
            return Ok(Err(unprocessable(
                ReasonCode::SpecInvalid,
                "missing template.attest scope",
            )));
        }
        // Request-boundary key validation (R7-05): only stable-identifier
        // keys may enter durable identity material — bytes that are not
        // legitimate URI path segments (control bytes such as NUL) are
        // rejected before any lookup, commit or hash.
        if let Err(e) = shaula_core::auth::validate_profile_key_for_fleet_ref(key) {
            return Ok(Err(unprocessable(e.code, e.summary)));
        }
        if let Err(e) = shaula_core::auth::validate_attestation_key(&payload.attestation_key) {
            return Ok(Err(unprocessable(e.code, e.summary)));
        }
        let now = self.now_ms();
        // The subject is the TYPED canonical struct (R7-04): its strict
        // shape was enforced at the request boundary, so what reaches the
        // durable record is always this bounded, sanitized serialization —
        // never arbitrary request JSON.
        let subject_json = serde_json::to_string(&payload.subject)
            .map_err(|e| CoreError::new(ReasonCode::Internal, e.to_string()))?;
        let subject_digest = request_hash(&[subject_json.as_bytes()]);

        // REPLAY PRE-CHECK FIRST (R6-07, spec 0005 §5): an exact replay
        // of an already-accepted request must reach the immutable
        // historical record and return the original result WITHOUT
        // re-running the current admission authority — replacing the
        // engine binary must not turn a recorded 201 into a 422. Only a
        // NEW identity goes through the authority below.
        if let Some(existing) = self
            .store
            .attestation_get(key, revision, &payload.attestation_key)
            .await?
        {
            let identical = existing.subject_json == subject_json
                && existing.result == payload.result
                && existing.evidence_digest == payload.evidence_digest
                && existing.suite_name.as_deref() == Some(payload.suite.0.as_str())
                && existing.suite_version.as_deref() == Some(payload.suite.1.as_str())
                && existing.completed_at == payload.completed_at;
            return Ok(if identical {
                Ok(payload.attestation_key.clone())
            } else {
                Err(MutationError::IdempotencyConflict)
            });
        }

        let Some(candidate) = self.store.template_revision_get(key, revision).await? else {
            return Ok(Err(MutationError::NotFound));
        };
        // Recompute the expected subject from stored authority; a submitted
        // field can only MATCH it, never redefine it (spec 0005 §5.1). A
        // MISMATCH is durable evidence now (R6-08): it is stored and
        // audited with `subject_verified=false`, never activated. A
        // genuine STORAGE failure is NOT a verdict — it propagates as a
        // 5xx instead of being recorded as a lie about why verification
        // failed.
        let engine_binary_digest = self.engine_binary_digest()?;
        let subject_verified = match verify_attestation_subject(
            self.store.as_ref(),
            &candidate,
            &payload,
            &engine_binary_digest,
        )
        .await
        {
            Ok(()) => true,
            Err(e) if e.code == ReasonCode::StorageUnavailable => return Err(e),
            Err(_) => false,
        };

        // The persisted identity binds the FULL path scope (R6-06):
        // Profile, Revision and the URI attestation key — unrelated
        // profiles reusing the same URI key never collide.
        let record = AttestationRecord {
            id: shaula_core::registry::attestation_record_id(
                key,
                revision,
                &payload.attestation_key,
            ),
            attestation_key: payload.attestation_key.clone(),
            profile_key: key.to_string(),
            revision,
            subject_json,
            subject_digest,
            result: payload.result.clone(),
            evidence_digest: payload.evidence_digest.clone(),
            suite: payload.suite.clone(),
            completed_at: payload.completed_at,
            subject_verified,
            // Only a passed, subject-VERIFIED attestation on the
            // still-desired Ready candidate may gate Ready -> Active;
            // the in-commit activation is best-effort so a stale (no
            // longer desired) but correct attestation stays durable and
            // audited instead of vanishing (R6-08).
            activate: subject_verified && payload.result == "passed",
        };
        match self
            .store
            .commit_attestation(record, actor.name.clone(), now)
            .await?
        {
            // Created, RecordedNotActivated and Replayed all answer with
            // the same stable attestation key — the replay never
            // re-activates and a non-activatable record is still 201
            // evidence.
            Ok(_) => Ok(Ok(payload.attestation_key.clone())),
            Err(mutation) => Ok(Err(mutation)),
        }
    }
}
