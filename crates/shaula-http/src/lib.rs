//! Loopback-only Axum adapter implementing the v1 control-plane surface.
//!
//! Strict-warnings note: the router helpers carry an axum Response as
//! their error payload and convert it into a real HTTP response at the
//! call boundary. Boxing every helper signature would add indirection
//! without behavioral benefit, so the size lint is deliberately allowed
//! at the crate root.
#![allow(clippy::result_large_err)]

#[cfg(test)]
extern crate self as shaula_http;

pub mod dto;
pub mod oidc;
pub mod problem;
pub mod router;
pub mod server;
pub mod state_backend;
mod web;

pub use router::build_router;
pub use server::{loopback_policy as verify_loopback, ServerConfig};
