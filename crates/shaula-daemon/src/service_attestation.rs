//! Attestation-subject recomputation (spec 0005 §5.1), split from
//! `service_validation.rs` to keep every file within the 400-line
//! limit (AGENTS.md).

use shaula_core::error::{CoreResult, ReasonCode};
use shaula_core::registry::ControlPlaneStore;

/// Recomputes the expected attestation subject from stored authority and
/// verifies every submitted member matches (spec 0005 §5.1). The
/// submitted subject is TYPED (R7-04): its shape was already strictly
/// validated at the request boundary, so only genuine member mismatches
/// reach here. `engine_binary_digest` is the SHA-256 of THE daemon's
/// configured engine binary — the exact implementation the conformance
/// run must have exercised.
///
/// Failure semantics (R7-03/R8-02): a MISMATCH is a verdict the caller
/// may record as durable evidence, but an unreadable, missing OR CORRUPT
/// published authority (manifest / dependency lock of an ADMITTED
/// artifact — including a readable-but-invalid document) is a retryable
/// STORAGE error (`ReasonCode::StorageUnavailable`) — without a usable
/// authority no mismatch fact may ever be concluded or persisted.
pub(crate) async fn verify_attestation_subject<S: ControlPlaneStore + ?Sized>(
    store: &S,
    candidate: &shaula_core::registry::TemplateRevisionRow,
    payload: &shaula_core::registry::AttestationPut,
    engine_binary_digest: &str,
) -> CoreResult<()> {
    use shaula_core::error::CoreError;

    let check_str = |submitted: &str, expected: &str, member: &str| -> CoreResult<()> {
        if submitted == expected {
            Ok(())
        } else {
            Err(CoreError::new(
                ReasonCode::TemplateInvalid,
                format!("attestation subject {member} mismatch"),
            ))
        }
    };
    // An ADMITTED candidate's authority files were published at
    // admission; their absence now is an operational fault, not a
    // property of the submission (R7-03).
    let storage = |what: &str| -> CoreError {
        CoreError::new(
            ReasonCode::StorageUnavailable,
            format!("published {what} authority unavailable"),
        )
    };

    let subject = &payload.subject;
    check_str(
        &subject.bindings_digest,
        candidate.bindings_digest.as_deref().unwrap_or_default(),
        "bindings_digest",
    )?;
    check_str(
        &subject.artifact_digest,
        &candidate.artifact_digest,
        "artifact_digest",
    )?;
    check_str(
        &subject.platform,
        candidate.platform.as_deref().unwrap_or_default(),
        "platform",
    )?;
    check_str(
        &subject.bindings_contract,
        candidate.bindings_contract.as_deref().unwrap_or_default(),
        "bindings_contract",
    )?;

    let expected_lock = store
        .artifact_lock_digest(&candidate.artifact_digest)
        .await?
        .ok_or_else(|| storage("dependency lock"))?;
    check_str(
        &subject.dependency_lock_digest,
        &expected_lock,
        "dependency_lock_digest",
    )?;

    // The admitted artifact manifest is THE authority for the engine and
    // the runtime surface: every remaining subject member must MATCH it,
    // never merely look well-formed (spec 0005 §5.1, ARD-0008/0009).
    let manifest_text = store
        .artifact_manifest(&candidate.artifact_digest)
        .await?
        .ok_or_else(|| storage("manifest"))?;
    // A manifest that was ADMITTED (validated at publication) but now
    // fails to parse or validate is server-side authority CORRUPTION
    // (R8-02): without the authority no mismatch may ever be concluded,
    // so it is a retryable STORAGE error, not a subject verdict.
    let manifest: shaula_core::template::ProfileManifest = serde_yaml::from_str(&manifest_text)
        .map_err(|e| {
            CoreError::new(
                ReasonCode::StorageUnavailable,
                format!("admitted manifest unreadable: {e}"),
            )
        })?;
    manifest.validate().map_err(|e| {
        CoreError::new(
            ReasonCode::StorageUnavailable,
            format!("admitted manifest no longer valid: {e}"),
        )
    })?;

    check_str(
        &subject.engine.kind,
        &manifest.runtime.engine,
        "engine.kind",
    )?;
    check_str(
        &subject.engine.required_version,
        &manifest.runtime.required_version,
        "engine.required_version",
    )?;

    // The conformance run's ACTUAL engine version must be pinned in the
    // subject and must SATISFY the manifest required-version constraint -
    // a range string alone proves nothing about what ran.
    if subject.engine.version.is_empty() {
        return Err(CoreError::new(
            ReasonCode::TemplateInvalid,
            "attestation subject engine.version missing",
        ));
    }
    shaula_core::template::version_satisfies(
        &subject.engine.version,
        &manifest.runtime.required_version,
    )
    .map_err(|e| {
        CoreError::new(
            ReasonCode::TemplateInvalid,
            format!("attestation subject engine.version rejected: {e}"),
        )
    })?;

    // Exact set equality (order-insensitive) with the manifest's admitted
    // images — a formatted-but-foreign image can never pass.
    let mut subject_images = subject.runner_image_digests.clone();
    subject_images.sort();
    subject_images.dedup();
    let mut admitted_images = manifest.runner_image_digests.clone();
    admitted_images.sort();
    if subject_images != admitted_images {
        return Err(CoreError::new(
            ReasonCode::TemplateInvalid,
            "attestation subject runner images do not match the admitted manifest",
        ));
    }

    check_str(
        &subject.runtime_policy_digest,
        &manifest.runtime_policy_digest,
        "runtime_policy_digest",
    )?;

    // The EXACT engine binary the harness ran must be THE daemon's own
    // configured binary — a hash of some other build proves nothing
    // about what this daemon will execute (spec 0005 §5.1).
    if engine_binary_digest.is_empty() {
        return Err(CoreError::new(
            ReasonCode::TemplateInvalid,
            "engine binary digest authority unavailable",
        ));
    }
    check_str(
        &subject.engine.binary_digest,
        engine_binary_digest,
        "engine.binary_digest",
    )?;

    // The exact provider source/version/checksum set, recomputed from
    // the artifact's OWN lock file — never trusted from the submission
    // (spec 0005 §5.1: the registry recomputes from the artifact).
    let lock_text = store
        .artifact_lock_file(&candidate.artifact_digest)
        .await?
        .ok_or_else(|| storage("dependency lock"))?;
    // An UNPARSEABLE published lock is authority corruption, not a
    // subject verdict (R8-02, same rule as the manifest above): without
    // the recomputed provider set no mismatch may ever be concluded.
    let locked = shaula_core::lockfile::parse_lock_providers(&lock_text).map_err(|e| {
        CoreError::new(
            ReasonCode::StorageUnavailable,
            format!("published dependency lock unreadable: {}", e.summary),
        )
    })?;
    let mut expected_providers: Vec<(String, String, String)> = locked
        .iter()
        .map(|p| {
            (
                shaula_core::lockfile::provider_source(&p.source),
                p.version.clone(),
                p.checksums_digest.clone(),
            )
        })
        .collect();
    expected_providers.sort();
    let mut submitted_providers: Vec<(String, String, String)> = Vec::new();
    for provider in &subject.providers {
        // R5-03: a degenerate member is a REFUSAL, never a silently
        // dropped element — dropping would let a forged set shrink
        // until it matched a weaker authority.
        if provider.source.is_empty()
            || provider.version.is_empty()
            || provider.checksums_digest.is_empty()
        {
            return Err(CoreError::new(
                ReasonCode::TemplateInvalid,
                "attestation subject providers contains a degenerate member",
            ));
        }
        submitted_providers.push((
            provider.source.clone(),
            provider.version.clone(),
            provider.checksums_digest.clone(),
        ));
    }
    submitted_providers.sort();
    if submitted_providers != expected_providers {
        return Err(CoreError::new(
            ReasonCode::TemplateInvalid,
            "attestation subject providers do not match the artifact dependency lock",
        ));
    }

    // The suite is part of the subject AND the envelope: only the
    // reviewed conformance suite can gate activation.
    if payload.suite.0 != shaula_core::template::ACCEPTED_CONFORMANCE_SUITE.0
        || payload.suite.1 != shaula_core::template::ACCEPTED_CONFORMANCE_SUITE.1
    {
        return Err(CoreError::new(
            ReasonCode::TemplateInvalid,
            "attestation suite is not the accepted conformance suite",
        ));
    }
    check_str(
        &subject.suite.name,
        shaula_core::template::ACCEPTED_CONFORMANCE_SUITE.0,
        "suite.name",
    )?;
    check_str(
        &subject.suite.version,
        shaula_core::template::ACCEPTED_CONFORMANCE_SUITE.1,
        "suite.version",
    )?;

    Ok(())
}
