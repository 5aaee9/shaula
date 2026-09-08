//! Cancellation-safe renewal: requests wait for an independently owned exchange.
use super::{
    session_refresh::{Admission, Flight, RefreshInput, Refreshed},
    token_exchange, AuthError, Authenticated, Oidc,
};
use oauth2::TokenResponse;
use shaula_core::secret::SecretString;
use std::{
    sync::{atomic::Ordering, Arc},
    time::Instant,
};

impl Oidc {
    pub(super) async fn session_identity(
        self: &Arc<Self>,
        id: &str,
    ) -> Result<Authenticated, AuthError> {
        loop {
            let mut store = self.sessions.lock().await;
            let mut completion = match store.admission(id)? {
                Admission::Fresh(identity) => return Ok(identity),
                Admission::Wait(completion) => completion,
                Admission::Renew(input) => {
                    let permit = self.exchanges.clone().try_acquire_owned().map_err(|_| {
                        store.defer(id, AuthError::Capacity);
                        AuthError::Capacity
                    })?;
                    let flight = Flight::new();
                    let completion = flight.subscribe();
                    store.start(id, flight.clone())?;
                    let oidc = self.clone();
                    let id = id.to_owned();
                    // No await between marking the flight and spawning its owner.
                    // Dropping a request can never cancel a rotating-token exchange.
                    tokio::spawn(async move {
                        let _permit = permit;
                        let result = oidc.refresh_session(input).await;
                        match &result {
                            Ok(_) => oidc.login_available.store(true, Ordering::Relaxed),
                            Err(AuthError::Provider) => {
                                oidc.login_available.store(false, Ordering::Relaxed)
                            }
                            _ => {}
                        }
                        let result = oidc.sessions.lock().await.finish(&id, &flight, result);
                        flight.finish(result);
                    });
                    completion
                }
            };
            drop(store);
            let result = *completion
                .wait_for(Option::is_some)
                .await
                .map_err(|_| AuthError::Unauthorized)?;
            result.ok_or(AuthError::Unauthorized)??;
            // A successful completion is not authority: logout may have removed it.
            // Read the still-registered session before forwarding this request.
        }
    }

    async fn refresh_session(&self, input: RefreshInput) -> Result<Refreshed, AuthError> {
        let tokens = self.exchange_refresh(&input.grant.token).await?;
        let received = Instant::now();
        // Once the token endpoint succeeds, the old token may already be consumed.
        // Any failure to validate that result is terminal, including JWKS outages.
        let (identity, expiry) = self
            .refreshed_identity(
                tokens.extra_fields().id_token.as_deref(),
                &input.grant.binding,
                &input.identity,
            )
            .await
            .map_err(|_| AuthError::Unauthorized)?;
        if tokens.scopes().is_some_and(|scopes| {
            !scopes.iter().any(|scope| scope.as_str() == "openid")
                || scopes.iter().any(|scope| {
                    !input
                        .grant
                        .scopes
                        .iter()
                        .any(|original| original == scope.as_str())
                })
        }) {
            return Err(AuthError::Unauthorized);
        }
        let expires = token_exchange::expiry(&tokens, expiry, received)?;
        let token = tokens
            .refresh_token()
            .map(|token| SecretString::new(token.secret().as_str()));
        Ok(Refreshed {
            identity,
            expires,
            token,
        })
    }
}
