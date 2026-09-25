//! Typed Auth requests enter the shared conditional publication protocol.

use super::conditional_put::{auth::Auth, Request};
use super::ControlPlane;
use shaula_core::error::CoreResult;
use shaula_core::registry::{Actor, AuthProfilePut, MutationAccepted, MutationError};

impl ControlPlane {
    pub(crate) async fn auth_put_impl(
        &self,
        actor: &Actor,
        key: &str,
        payload: AuthProfilePut,
        if_none_match: bool,
        if_match: Option<(String, i64)>,
        idempotency_key: Option<String>,
    ) -> CoreResult<Result<MutationAccepted, MutationError>> {
        self.conditional_put(
            Request {
                actor,
                key,
                create: if_none_match,
                expected: if_match,
                idempotency_key,
            },
            || Auth::prepare(key, payload),
        )
        .await
    }
}
