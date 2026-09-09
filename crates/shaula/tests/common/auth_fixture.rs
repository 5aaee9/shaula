//! Validated GitHub App fixtures shared by HTTP and lifecycle integration tests.

use shaula_core::auth_context::{AccountBinding, RepositorySelection};
use shaula_core::auth_policy::AccountKind;
use shaula_core::error::CoreResult;
use shaula_core::registry::{
    auth_dependent_set_fingerprint, AuthCheckedFleet, AuthPromotion, AuthPromotionOutcome,
    AuthValidationSnapshot, ControlPlaneStore,
};

/// Supplies the explicit verification result normally produced by the GitHub
/// validator; it does not bypass the store's policy/dependency/CAS checks.
pub async fn promote(
    store: &dyn ControlPlaneStore,
    key: &str,
    revision: i64,
    now: i64,
) -> CoreResult<()> {
    let dependents = store.auth_live_dependents(key).await?;
    let snapshot = AuthValidationSnapshot {
        candidate: (key.into(), revision),
        dependent_set: auth_dependent_set_fingerprint(&dependents),
        checked_fleets: dependents
            .iter()
            .map(|d| AuthCheckedFleet {
                key: d.fleet_key.clone(),
                incarnation: d.incarnation.clone(),
                revision: d.revision,
                fence: d.fence,
            })
            .collect(),
        identities: vec![],
    };
    let outcome = store
        .auth_apply_validation_v2(
            key,
            revision,
            true,
            None,
            now,
            Some(AuthPromotion {
                bindings: vec![AccountBinding {
                    account_id: 100,
                    account_kind: AccountKind::Organization,
                    login: "example-org".into(),
                    installation_id: 11,
                    repository_selection: RepositorySelection::All,
                    validated_at_ms: now,
                }],
                snapshot_json: serde_json::to_string(&snapshot).map_err(|error| {
                    shaula_core::error::CoreError::new(
                        shaula_core::error::ReasonCode::Internal,
                        error.to_string(),
                    )
                })?,
            }),
        )
        .await?;
    assert_eq!(outcome, AuthPromotionOutcome::Promoted);
    Ok(())
}
