//! SeaORM entities grouped by aggregate domain. Entities stay private to
//! the store crate per the architecture contract; only typed repositories
//! escape.

pub mod auth;
pub mod fleet;
pub mod gitlab;
pub mod jobs;
pub mod lifecycle;
pub mod operation_logs;
pub mod shared;
pub mod template;
pub mod template_pool;
