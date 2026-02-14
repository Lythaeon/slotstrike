#![allow(clippy::missing_errors_doc, clippy::must_use_candidate)]

pub mod adapters;
pub mod app;
pub mod domain;
pub mod ports;
pub mod slices;

pub const MAX_RETRIES: usize = 1_000;
