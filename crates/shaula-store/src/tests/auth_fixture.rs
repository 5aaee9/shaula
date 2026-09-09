//! Publication evidence for ordinary persistence tests using a GitHub App.
use shaula_core::auth_context::{AccountBinding, RepositorySelection};
use shaula_core::auth_policy::AccountKind;
use shaula_core::registry::{
    auth_dependent_set_fingerprint, AuthCheckedFleet, AuthPromotion, AuthValidationSnapshot,
};

pub(super) const POLICY: &str = r#"{"selectors":[{"kind":"organization","owner":"example-org"}]}"#;
pub(super) const SPEC: &str =
    r#"{"github":{"target":{"kind":"organization","owner":"example-org"}}}"#;

pub(super) async fn promotion(
    store: &crate::Store,
    key: &str,
    revision: i64,
) -> crate::StoreResult<AuthPromotion> {
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
    Ok(AuthPromotion {
        bindings: vec![AccountBinding {
            account_id: 100,
            account_kind: AccountKind::Organization,
            login: "example-org".into(),
            installation_id: 11,
            repository_selection: RepositorySelection::All,
            validated_at_ms: 1,
        }],
        snapshot_json: serde_json::to_string(&snapshot)
            .map_err(|error| crate::StoreError::Corrupt(error.to_string()))?,
    })
}
