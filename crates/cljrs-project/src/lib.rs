//! `cljrs-project` — the project layer: what a clojurust *project* is made of.
//!
//! Two things, which every consumer needs together:
//!
//! * [`config`] — the `cljrs.edn` model: source paths, dependencies, aliases,
//!   the `:rust` native-crate declaration, and trusted commit signers.
//! * [`vcs`] — the git operations that turn a declared dependency into files on
//!   disk: fetch, cache, worktree checkout, and commit-signature verification.
//!
//! The `vcs` module is native-only. Gitoxide and the signature verifiers do not
//! build for `wasm32`, and a browser runtime has no git dependencies to
//! resolve, so the module (and its dependencies) are compiled out there.

pub mod config;
#[cfg(not(target_arch = "wasm32"))]
pub mod vcs;
