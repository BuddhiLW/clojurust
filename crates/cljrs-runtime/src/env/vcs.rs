//! The runtime's interface to version control.
//!
//! Versioned symbol resolution (`ns/name@commit`) and commit-signature
//! verification are the only two places where the interpreter touches git.
//! Calling `cljrs_project::vcs` directly from here would link gitoxide, rPGP,
//! `ssh-key` — and, through gix's blocking http transport, reqwest/hyper/rustls
//! — into *every* embedding of the interpreter, including ones that never
//! resolve a versioned var.
//!
//! So the runtime talks to a [`VcsProvider`] trait object instead:
//!
//! * With the default `deps` feature on, [`GlobalEnv::new`] installs
//!   [`ProjectVcs`] — the `cljrs-project`-backed implementation — so behaviour
//!   is exactly what it was before the split and no downstream crate changes.
//! * Built with `--no-default-features`, no provider is installed:
//!   [`VcsProvider::find_repo_root`] never gets asked, so a source file is
//!   treated as "not in a git repository", versioned resolution falls back to
//!   embedded (AOT) sources, and gix/pgp/ssh-key are not compiled at all.
//!
//! The `GlobalEnv` field holding the provider exists in both configurations,
//! so the struct's layout does not change with the feature — an embedder that
//! links a `deps`-less runtime alongside a `deps`-ful one is not exposed to a
//! feature-unification surprise.
//!
//! [`GlobalEnv::new`]: crate::env::env::GlobalEnv::new

use std::path::{Path, PathBuf};

use cljrs_project::config::TrustedSigner;

/// Why a commit-signature check did not succeed.
#[derive(Debug)]
pub enum SignatureFailure {
    /// The commit is unsigned, its signature is invalid, or the signing key is
    /// not in the trusted set.  Surfaces to user code as
    /// `EvalError::CommitSignatureVerificationFailed`.
    Untrusted { commit: String, reason: String },
    /// The check could not be carried out at all (malformed hash, unreadable
    /// repository, …).  Surfaces as a plain runtime error.
    Error(String),
}

/// The git operations the runtime needs, as an interface.
///
/// Implementations must be cheap to clone-by-`Arc` and safe to call from any
/// thread: a provider is shared by every thread evaluating in a `GlobalEnv`.
pub trait VcsProvider: Send + Sync {
    /// Walk upward from `start` (a file or directory) to the enclosing git
    /// working-tree root, or `None` when `start` is not inside a repository.
    fn find_repo_root(&self, start: &Path) -> Option<PathBuf>;

    /// Read `rel_path` (relative to `repo_root`) as it existed at `commit`.
    fn file_at_commit(
        &self,
        repo_root: &Path,
        rel_path: &str,
        commit: &str,
    ) -> Result<String, String>;

    /// Verify that `commit` in `repo_root` carries a cryptographically valid
    /// signature made by one of the keys installed by
    /// [`load_trusted_signers`](VcsProvider::load_trusted_signers).
    fn verify_commit_signature(
        &self,
        repo_root: &Path,
        commit: &str,
    ) -> Result<(), SignatureFailure>;

    /// Install the set of keys trusted to sign versioned dependency commits,
    /// replacing any previously installed set.  Returns the number of keys
    /// successfully loaded; malformed or unreadable keys are warned about
    /// rather than aborting.
    fn load_trusted_signers(&self, signers: &[TrustedSigner]) -> usize;
}

/// The provider installed by [`GlobalEnv::new`], or `None` in builds that
/// carry no VCS implementation.
///
/// [`GlobalEnv::new`]: crate::env::env::GlobalEnv::new
pub fn default_provider() -> Option<std::sync::Arc<dyn VcsProvider>> {
    #[cfg(all(feature = "deps", not(target_arch = "wasm32")))]
    {
        Some(std::sync::Arc::new(ProjectVcs::new()))
    }
    #[cfg(not(all(feature = "deps", not(target_arch = "wasm32"))))]
    {
        None
    }
}

// ── cljrs-project-backed implementation ───────────────────────────────────────

/// [`VcsProvider`] implemented on top of `cljrs_project::vcs` (gitoxide for the
/// git side, rPGP / `ssh-key` for signatures).
#[cfg(all(feature = "deps", not(target_arch = "wasm32")))]
pub struct ProjectVcs {
    /// Public keys trusted to sign versioned dependency commits, built from the
    /// `:trusted-signers` config.  Empty until `load_trusted_signers` runs, in
    /// which case every signature check fails as untrusted.
    trusted: std::sync::RwLock<std::sync::Arc<cljrs_project::vcs::TrustedKeys>>,
}

#[cfg(all(feature = "deps", not(target_arch = "wasm32")))]
impl Default for ProjectVcs {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(all(feature = "deps", not(target_arch = "wasm32")))]
impl ProjectVcs {
    pub fn new() -> Self {
        Self {
            trusted: std::sync::RwLock::new(std::sync::Arc::new(
                cljrs_project::vcs::TrustedKeys::new(),
            )),
        }
    }
}

#[cfg(all(feature = "deps", not(target_arch = "wasm32")))]
impl VcsProvider for ProjectVcs {
    fn find_repo_root(&self, start: &Path) -> Option<PathBuf> {
        cljrs_project::vcs::find_repo_root(start)
    }

    fn file_at_commit(
        &self,
        repo_root: &Path,
        rel_path: &str,
        commit: &str,
    ) -> Result<String, String> {
        cljrs_project::vcs::get_file_at_commit(repo_root, rel_path, commit)
            .map_err(|e| e.to_string())
    }

    fn verify_commit_signature(
        &self,
        repo_root: &Path,
        commit: &str,
    ) -> Result<(), SignatureFailure> {
        let trusted = self.trusted.read().unwrap().clone();
        cljrs_project::vcs::verify_commit_signature(repo_root, commit, &trusted).map_err(
            |e| match e {
                cljrs_project::vcs::VcsError::SignatureVerificationFailed { commit, reason } => {
                    SignatureFailure::Untrusted { commit, reason }
                }
                other => SignatureFailure::Error(other.to_string()),
            },
        )
    }

    fn load_trusted_signers(&self, signers: &[TrustedSigner]) -> usize {
        let mut keys = cljrs_project::vcs::TrustedKeys::new();
        let mut loaded = 0usize;
        for signer in signers {
            let result = match signer {
                TrustedSigner::Inline(text) => keys.add_key_text(text),
                TrustedSigner::File(path) => match std::fs::read_to_string(path) {
                    Ok(text) => keys.add_key_text(&text),
                    Err(e) => {
                        eprintln!(
                            "cljrs: warning: could not read trusted signer key {}: {e}",
                            path.display()
                        );
                        continue;
                    }
                },
            };
            match result {
                Ok(()) => loaded += 1,
                Err(e) => eprintln!("cljrs: warning: invalid trusted signer key: {e}"),
            }
        }
        *self.trusted.write().unwrap() = std::sync::Arc::new(keys);
        loaded
    }
}
