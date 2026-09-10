//! The cargo gate over `tests/cljrs`, the cljrs-owned clojure.test tree.
//!
//! That tree is the language-surface half of this project's tests: what
//! `deftype`, `defmulti`, protocol impl positions and reader metadata *mean*,
//! written in the language rather than as Rust string literals asserting on
//! the printed form of a value. CI drives it directly, but a contributor
//! running a plain `cargo test` would otherwise never see it — so this test
//! shells out to the built binary via `CARGO_BIN_EXE_cljrs` and fails the Rust
//! suite when a language-level assertion breaks.
//!
//! Only the interpreter leg runs here. The AOT leg (`cljrs compile --test`)
//! invokes `cargo` to build a harness crate, which is not something a test
//! already running under cargo should do; CI runs that leg separately.

use std::path::{Path, PathBuf};
use std::process::Command;

/// `tests/cljrs`, resolved from this crate rather than from the cwd.
fn tree() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/cljrs")
}

#[test]
fn the_self_hosted_tree_passes_under_the_interpreter() {
    let tree = tree();
    assert!(
        tree.is_dir(),
        "the self-hosted test tree is missing at {}",
        tree.display()
    );

    let out = Command::new(env!("CARGO_BIN_EXE_cljrs"))
        .arg("test")
        .arg("--src-path")
        .arg(&tree)
        .output()
        .expect("run cljrs binary");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        out.status.success(),
        "cljrs test --src-path {} failed\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}",
        tree.display()
    );

    // A tree that discovered nothing also exits 0. Assert it actually ran
    // something, so a rename that orphans the whole tree is a failure and not
    // a silent pass.
    assert!(
        stdout.contains("All tests passed."),
        "expected a passing summary\n--- stdout ---\n{stdout}"
    );

    // The per-namespace lines legitimately include "Ran 0 tests" — a fixture
    // namespace carries definitions and no `deftest`. The count that matters
    // is the run-wide summary, "Ran N tests containing M assertions across …".
    let ran = stdout
        .lines()
        .find(|l| l.contains("assertions across"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|n| n.parse::<u32>().ok())
        .unwrap_or_else(|| panic!("no run-wide summary line\n--- stdout ---\n{stdout}"));

    assert!(
        ran > 0,
        "the tree discovered no tests — did the layout or the extension change?\
         \n--- stdout ---\n{stdout}"
    );
}
