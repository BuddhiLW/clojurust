//! Internal command implementations for the `cljrs` binary.
//!
//! Each module owns one top-level subcommand: its clap `Subcommand` enum, its
//! dispatch function, and any helpers that exist only to serve it. `main.rs`
//! wires the enums into the top-level `Commands` and calls the dispatchers.

pub mod ir;
