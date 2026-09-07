//! SeaORM entities grouped by aggregate domain. Entities stay private to
//! the store crate per the architecture contract; only typed repositories
//! escape.

pub mod auth;
pub mod fleet;
pub mod lifecycle;
pub mod shared;
pub mod template;
