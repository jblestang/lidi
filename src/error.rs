//! Shared helpers for ANSSI R12 (`std::error::Error` on crate error types).

pub trait DiodeError: std::error::Error + Send + Sync + 'static {}

impl<T> DiodeError for T where T: std::error::Error + Send + Sync + 'static {}
