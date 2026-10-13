//! GitLab runner control-plane adapter (spec 0043 §3).
//!
//! Implements only [`shaula_core::ports::gitlab::GitlabPoolPort`]. Ways this
//! adapter can fail, and the behavior each one is held to:
//!
//! 1. Registration effect lands but the response is lost (transport error,
//!    2xx other than 201, 5xx, unreadable or incomplete body): reported as
//!    `Uncertain`; the body is never projected because it may hold a token.
//! 2. A runner or access token reaches an error summary: summaries redact
//!    the access token and are truncated; registration bodies are never read
//!    for error text.
//! 3. A redirect forwards `PRIVATE-TOKEN` to another origin: redirects are
//!    disabled, so a renamed scope path fails closed instead.
//! 4. A scope path is renamed and re-created as a different group/project:
//!    the resolved numeric id is compared with the pinned id and the full
//!    path read back; any mismatch is `PermissionDenied`.
//! 5. A same-description runner belongs to someone else: inventory returns
//!    `maintenance_note` from the per-runner detail read; ownership is the
//!    caller's joint check, never inferred from the description alone.
//! 6. Pagination drifts while runners are created/deleted: a repeated id or
//!    an exhausted page/time budget is `Unavailable`, not a partial list.
//! 7. Demand is silently zero because the token cannot see the anchor's
//!    jobs: the same GraphQL request reads the anchor's `updateRunner`
//!    permission, pause state and matching attributes; the caller proves it.
//! 8. GraphQL reports failure inside HTTP 200 (`errors`): `Unavailable`.
//! 9. `404` on a runner because the scope became hidden rather than the
//!    runner being deleted: every runner read/write first re-proves scope
//!    access, so a later 404 is absence.
//! 10. Pause/delete transport errors: `RequestUncertain`; the caller rereads.
//! 11. Oversized or unbounded responses: streamed bodies are capped.
//! 12. An unverified server version: refused before any registration.

mod body;
mod client;
mod error;
mod graphql;
mod models;
mod port;
mod runners;

pub use client::GitlabClient;
pub use error::GitlabError;

pub mod validation;
