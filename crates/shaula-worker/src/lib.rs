//! A Worker owns one sequential Generation lifecycle. The daemon supplies
//! material and safety permits; it never sends individual Terraform commands.
mod client;
mod intent;
mod logs;
mod run;

pub use client::WorkerClient;
pub use run::run;
