//! Bounded optional projection. Domain transactions never await this writer.
mod hub;
mod read;
mod writer;
pub(crate) use hub::Hub;
pub(crate) use writer::{guard_valid, subject_key};

#[cfg(test)]
pub(crate) use writer::collect;
