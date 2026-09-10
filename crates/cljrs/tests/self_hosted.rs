//! Runs `tests/cljrs`, the cljrs-owned clojure.test tree, under the built
//! binary. Interpreter leg only: the AOT leg invokes cargo itself.

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

    // A tree that discovered nothing also exits 0.
    assert!(
        stdout.contains("All tests passed."),
        "expected a passing summary\n--- stdout ---\n{stdout}"
    );

    // Per-namespace lines may say "Ran 0 tests" (a fixture ns); the run-wide
    // summary is the count that matters.
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
