// ---- Persistence ports ----
//
// The daemon sees storage only through these ports. Every composite
// mutation is one atomic unit inside the store: revision + change + audit
// + outbox + idempotency facts commit together or not at all.
//
// The control plane is split into resource-cohesive sub-traits; every
// method is required so a forgotten implementation fails to compile
// instead of silently succeeding.

mod artifact;
mod auth_profile;
mod capacity;
mod fleet;
mod handoff;
mod profile;
mod support;
mod template;
mod template_pool;

pub use artifact::ArtifactStore;
pub use auth_profile::AuthProfileStore;
pub use capacity::CapacityStore;
pub use fleet::FleetStore;
pub use handoff::HandoffStore;
pub use profile::ProfileStore;
pub use support::AdmissionSupportStore;
pub use template::TemplateStore;
pub use template_pool::TemplatePoolStore;

pub use super::mutation_facts::MutationFacts;

/// The full desired-state store used by admission: every resource port
/// plus retained execution authority. Implemented automatically for any
/// type implementing all sub-traits.
pub trait ControlPlaneStore:
    crate::registry::AuthExecutionStore
    + FleetStore
    + CapacityStore
    + TemplateStore
    + TemplatePoolStore
    + AuthProfileStore
    + HandoffStore
    + ArtifactStore
    + ProfileStore
    + AdmissionSupportStore
    + Send
    + Sync
{
}

impl<T> ControlPlaneStore for T where
    T: crate::registry::AuthExecutionStore
        + FleetStore
        + CapacityStore
        + TemplateStore
        + TemplatePoolStore
        + AuthProfileStore
        + HandoffStore
        + ArtifactStore
        + ProfileStore
        + AdmissionSupportStore
        + Send
        + Sync
        + ?Sized
{
}

pub use super::store_rows::{FleetHead, FleetRevisionRow, ProfileHead, TemplateRevisionRow};

/// One immutable auth revision row lives in [`super::auth_port`].
pub use super::auth_port::{
    AuthDependentTarget, AuthHandoffRow, AuthRevisionRow, FleetAuthContextRow,
};
