//! SeaORM entities grouped by aggregate domain. Entities stay private to
//! the store crate per the architecture contract; only typed repositories
//! escape.

pub mod access_tokens;
pub mod auth;
pub mod diagnostics;
pub mod fleet;
pub mod gitlab;
pub mod http_state;
pub mod jobs;
pub mod lifecycle;
pub mod listener;
pub mod operation_logs;
pub mod shared;
pub mod template;
pub mod template_pool;
