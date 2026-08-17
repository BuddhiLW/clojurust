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
//!
//! It is also gated behind the default-on `vcs` feature, so a consumer that
//! only needs the config model — the interpreter, an editor plugin, a
//! sandboxed evaluator — can turn it off and link none of gitoxide, rPGP or
//! `ssh-key`. Network fetch/clone is a further step up: it needs `vcs-net`,
//! which is what selects gix's http transport and so the reqwest/rustls stack.

pub mod config;
#[cfg(all(feature = "vcs", not(target_arch = "wasm32")))]
pub mod vcs;
