//! Pure session-store transitions. Provider I/O is owned by `renewal`.
use super::{
    sessions::{RefreshGrant, Sessions},
    AuthError, Authenticated,
};
use shaula_core::secret::SecretString;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::watch;

const BACKOFF: Duration = Duration::from_secs(10);
type Completion = Option<Result<(), AuthError>>;

pub(super) struct Flight {
    complete: watch::Sender<Completion>,
}

impl Flight {
    pub fn new() -> Arc<Self> {
        let (complete, _) = watch::channel(None);
        Arc::new(Self { complete })
    }

    pub fn subscribe(&self) -> watch::Receiver<Completion> {
        self.complete.subscribe()
    }

    pub fn finish(&self, result: Result<(), AuthError>) {
        self.complete.send_replace(Some(result));
    }
}

pub(super) enum RenewalState {
    Idle,
    Running(Arc<Flight>),
    Backoff { until: Instant, error: AuthError },
}

pub(super) struct RefreshInput {
    pub identity: Authenticated,
    pub grant: RefreshGrant,
}

pub(super) struct Refreshed {
    pub identity: Authenticated,
    pub expires: Instant,
    pub token: Option<SecretString>,
}

pub(super) enum Admission {
    Fresh(Authenticated),
    Wait(watch::Receiver<Completion>),
    Renew(RefreshInput),
}

impl Sessions {
    pub(super) fn admission(&mut self, id: &str) -> Result<Admission, AuthError> {
        self.evict();
        let session = self.sessions.get(id).ok_or(AuthError::Unauthorized)?;
        let now = Instant::now();
        if session.expires > now && session.idle > now {
            return self.get(id).map(Admission::Fresh);
        }
        let grant = session.refresh.as_ref().ok_or(AuthError::Unauthorized)?;
        match &session.renewal {
            RenewalState::Running(flight) => Ok(Admission::Wait(flight.subscribe())),
            RenewalState::Backoff { until, error } if *until > now => Err(*error),
            RenewalState::Idle | RenewalState::Backoff { .. } => {
                Ok(Admission::Renew(RefreshInput {
                    identity: session.identity.clone(),
                    grant: grant.clone(),
                }))
            }
        }
    }

    pub(super) fn start(&mut self, id: &str, flight: Arc<Flight>) -> Result<(), AuthError> {
        let session = self.sessions.get_mut(id).ok_or(AuthError::Unauthorized)?;
        session.renewal = RenewalState::Running(flight);
        Ok(())
    }

    pub(super) fn defer(&mut self, id: &str, error: AuthError) {
        if let Some(session) = self.sessions.get_mut(id) {
            session.renewal = RenewalState::Backoff {
                until: Instant::now() + BACKOFF,
                error,
            };
        }
    }

    /// Membership and exact-flight fencing prevent logout/replacement resurrection.
    pub(super) fn finish(
        &mut self,
        id: &str,
        flight: &Arc<Flight>,
        result: Result<Refreshed, AuthError>,
    ) -> Result<(), AuthError> {
        let session = self.sessions.get_mut(id).ok_or(AuthError::Unauthorized)?;
        if !matches!(&session.renewal, RenewalState::Running(current) if Arc::ptr_eq(current, flight))
        {
            return Err(AuthError::Unauthorized);
        }
        match result {
            Ok(mut refreshed) if refreshed.expires > Instant::now() => {
                refreshed.identity.csrf.clone_from(&session.identity.csrf);
                session.identity = refreshed.identity;
                session.expires = refreshed.expires;
                session.idle = Instant::now() + Duration::from_secs(900);
                if let Some(token) = refreshed.token {
                    let grant = session.refresh.as_mut().ok_or(AuthError::Unauthorized)?;
                    grant.token = token;
                }
                session.renewal = RenewalState::Idle;
                Ok(())
            }
            Err(error @ (AuthError::Provider | AuthError::Capacity)) => {
                self.defer(id, error);
                Err(error)
            }
            Ok(_) | Err(_) => {
                self.remove(id);
                Err(AuthError::Unauthorized)
            }
        }
    }
}
