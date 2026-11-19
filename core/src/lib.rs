//! Core logic of OMSI-Tools-NG: configuration, OMSI text parsing, map scanning and cleanup.
//!
//! This crate has no UI. It is a Rust rewrite of the map scanner and cleanup features of
//! pietr26's OMSI-Tools (GPLv2).

pub mod cleanup;
pub mod config;
pub mod encoding;
pub mod maps;
pub mod paths;
pub mod scanner;

pub use config::{Config, GameDirs, Source};
