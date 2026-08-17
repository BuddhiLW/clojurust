//! The `VcsProvider` seam: what the runtime does with, without, and around a
//! git backend.
//!
//! These are the guarantees the `deps` feature rests on — a build with no
//! provider must degrade to "not in a git repository" rather than misbehave,
//! and must *fail* a signature check it cannot perform.

#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use cljrs_project::config::TrustedSigner;
use cljrs_runtime::env::env::{Env, GlobalEnv};
use cljrs_runtime::env::vcs::{SignatureFailure, VcsProvider};

fn make_globals() -> Arc<GlobalEnv> {
    cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
        .build()
        .expect("runtime")
        .into_globals()
}

/// A provider that claims every path is a repository and answers with canned
/// content, so the seam can be exercised without a git fixture.
struct StubVcs {
    signature_ok: bool,
}

impl VcsProvider for StubVcs {
    fn find_repo_root(&self, _start: &Path) -> Option<PathBuf> {
        Some(PathBuf::from("/stub/repo"))
    }

    fn file_at_commit(
        &self,
        _repo_root: &Path,
        rel_path: &str,
        commit: &str,
    ) -> Result<String, String> {
        Ok(format!("(def marker \"{rel_path}@{commit}\")"))
    }

    fn verify_commit_signature(
        &self,
        _repo_root: &Path,
        commit: &str,
    ) -> Result<(), SignatureFailure> {
        if self.signature_ok {
            Ok(())
        } else {
            Err(SignatureFailure::Untrusted {
                commit: commit.to_string(),
                reason: "stub rejects everything".to_string(),
            })
        }
    }

    fn load_trusted_signers(&self, signers: &[TrustedSigner]) -> usize {
        signers.len()
    }
}

/// An always-approving provider that counts how often it is actually asked,
/// so caching and cache invalidation are observable.
#[derive(Default)]
struct CountingVcs {
    verify_calls: std::sync::atomic::AtomicUsize,
}

impl CountingVcs {
    fn calls(&self) -> usize {
        self.verify_calls.load(Ordering::Relaxed)
    }
}

impl VcsProvider for CountingVcs {
    fn find_repo_root(&self, _start: &Path) -> Option<PathBuf> {
        Some(PathBuf::from("/stub/repo"))
    }

    fn file_at_commit(
        &self,
        _repo_root: &Path,
        _rel_path: &str,
        _commit: &str,
    ) -> Result<String, String> {
        Ok(String::new())
    }

    fn verify_commit_signature(
        &self,
        _repo_root: &Path,
        _commit: &str,
    ) -> Result<(), SignatureFailure> {
        self.verify_calls.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn load_trusted_signers(&self, signers: &[TrustedSigner]) -> usize {
        signers.len()
    }
}

/// With the default `deps` feature a provider is installed; without it, none is.
#[test]
fn default_provider_matches_feature() {
    let globals = make_globals();
    assert_eq!(globals.vcs().is_some(), cfg!(feature = "deps"));
}

/// Signature checking stays a no-op while `:verify-commit-signatures` is off,
/// provider or not.
#[test]
fn signature_check_is_noop_when_disabled() {
    let globals = make_globals();
    globals.set_vcs_provider(None);
    assert!(
        globals
            .check_commit_signature("/anywhere", "abc1234")
            .is_ok()
    );
}

/// A build that cannot verify must not silently accept: with verification
/// demanded and no provider, the check fails.
#[test]
fn signature_check_fails_without_provider() {
    let globals = make_globals();
    globals.set_vcs_provider(None);
    globals
        .verify_commit_signatures
        .store(true, Ordering::Relaxed);

    let err = globals
        .check_commit_signature("/anywhere", "abc1234")
        .expect_err("must not pass a check it cannot perform");
    assert!(
        format!("{err:?}").contains("no VCS provider"),
        "unexpected error: {err:?}"
    );
}

/// The installed provider is what decides the verdict.
#[test]
fn installed_provider_decides_signature_verdict() {
    let globals = make_globals();
    globals
        .verify_commit_signatures
        .store(true, Ordering::Relaxed);

    globals.set_vcs_provider(Some(Arc::new(StubVcs {
        signature_ok: false,
    })));
    assert!(
        globals
            .check_commit_signature("/stub/repo", "abc1234")
            .is_err()
    );

    globals.set_vcs_provider(Some(Arc::new(StubVcs { signature_ok: true })));
    assert!(
        globals
            .check_commit_signature("/stub/repo", "abc1234")
            .is_ok()
    );
}

/// A pass is cached, so a given `(repo, commit)` costs one verification per
/// session — but the cache belongs to the provider that filled it.  Swapping
/// the provider must drop those verdicts, or a permissive provider could
/// launder an approval for source a stricter one would reject.
#[test]
fn provider_swap_invalidates_cached_verdicts() {
    let globals = make_globals();
    globals
        .verify_commit_signatures
        .store(true, Ordering::Relaxed);

    let counting = Arc::new(CountingVcs::default());
    globals.set_vcs_provider(Some(counting.clone()));
    assert!(
        globals
            .check_commit_signature("/stub/repo", "abc1234")
            .is_ok()
    );
    assert!(
        globals
            .check_commit_signature("/stub/repo", "abc1234")
            .is_ok()
    );
    assert_eq!(counting.calls(), 1, "a pass should be cached");

    // Swap in a provider that rejects: the earlier approval must not carry
    // over to it.
    globals.set_vcs_provider(Some(Arc::new(StubVcs {
        signature_ok: false,
    })));
    assert!(
        globals
            .check_commit_signature("/stub/repo", "abc1234")
            .is_err(),
        "cached verdict from the previous provider leaked"
    );
}

/// Reloading the trusted-key set is the same hazard: the verdicts in the
/// cache were reached under the old keys.
#[test]
fn reloading_trusted_signers_invalidates_cached_verdicts() {
    let globals = make_globals();
    globals
        .verify_commit_signatures
        .store(true, Ordering::Relaxed);

    let counting = Arc::new(CountingVcs::default());
    globals.set_vcs_provider(Some(counting.clone()));
    assert!(
        globals
            .check_commit_signature("/stub/repo", "abc1234")
            .is_ok()
    );
    assert_eq!(counting.calls(), 1);

    globals.load_trusted_signers(&cljrs_project::config::DepsConfig {
        trusted_signers: vec![TrustedSigner::Inline("key-a".to_string())],
        ..Default::default()
    });
    assert!(
        globals
            .check_commit_signature("/stub/repo", "abc1234")
            .is_ok()
    );
    assert_eq!(
        counting.calls(),
        2,
        "new trust set must force re-verification"
    );
}

/// Trusted signers are routed to the provider, and are simply dropped when
/// there is none to consume them.
#[test]
fn trusted_signers_route_through_the_provider() {
    let globals = make_globals();
    let config = cljrs_project::config::DepsConfig {
        trusted_signers: vec![
            TrustedSigner::Inline("key-a".to_string()),
            TrustedSigner::Inline("key-b".to_string()),
        ],
        ..Default::default()
    };

    globals.set_vcs_provider(Some(Arc::new(StubVcs { signature_ok: true })));
    assert_eq!(globals.load_trusted_signers(&config), 2);

    globals.set_vcs_provider(None);
    assert_eq!(globals.load_trusted_signers(&config), 0);
}

/// Without a provider, a namespace loaded from disk records no repository —
/// the same state as a source tree that is genuinely not checked in, which is
/// what makes versioned resolution degrade rather than break.
#[test]
fn no_provider_means_no_repo_root() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("plainns.cljc"), "(def x 1)\n").expect("write source");

    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
        .source_paths(vec![dir.path().to_path_buf()])
        .build()
        .expect("runtime")
        .into_globals();
    globals.set_vcs_provider(None);

    let mut env = Env::new(globals.clone(), "user");
    let mut parser =
        cljrs_reader::Parser::new("(require 'plainns)".to_string(), "<test>".to_string());
    for form in parser.parse_all().expect("parse") {
        globals.eval(&form, &mut env).expect("require");
    }

    let ns = globals.get_or_create_ns("plainns");
    let ns = ns.get();
    assert!(
        ns.source_file.lock().unwrap().is_some(),
        "source file should still be recorded"
    );
    let repo_root = ns.git_repo_root.lock().unwrap().clone();
    assert!(
        repo_root.is_none(),
        "no provider must mean no repo root, got {repo_root:?}"
    );
}
