//! End-to-end tests for `cljrs compile --test` (`compile_test_harness`).
//!
//! Each test writes a small project (a source namespace and a clojure.test
//! namespace), compiles it to a standalone test binary, runs the binary, and
//! asserts on its output/exit status.  Also asserts that the harness actually
//! AOT-compiles namespaces (a `__cljrs_ns_init_*` initializer is linked and
//! registered) rather than bundling every source for interpretation.
//!
//! Gated behind the `aot_full_test` feature like the rest of the AOT e2e
//! suite:
//!
//! ```sh
//! cargo test -p cljrs-compiler --features aot_full_test --test test_harness_e2e
//! ```

#![cfg(feature = "aot_full_test")]

use std::path::Path;
use std::process::Command;
use std::sync::Mutex;

/// Serialize harness builds — each invokes `cargo build` in a harness project,
/// and concurrent cargo processes fight over the crates.io index lock.
static AOT_LOCK: Mutex<()> = Mutex::new(());

struct TestProject {
    dir: std::path::PathBuf,
}

impl TestProject {
    /// Lay out `src/` + `test/` under a fresh temp dir.
    fn new(name: &str, src_files: &[(&str, &str)], test_files: &[(&str, &str)]) -> Self {
        let dir = std::env::temp_dir().join(format!("cljrs_test_harness_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        for (rel, contents) in src_files {
            let path = dir.join("src").join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
        }
        for (rel, contents) in test_files {
            let path = dir.join("test").join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
        }
        TestProject { dir }
    }

    /// Compile the project's tests to a binary and run it, returning
    /// `(stdout, exit_success)`.
    fn compile_and_run(&self, name: &str) -> (String, bool) {
        let _guard = AOT_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let bin_path = self.dir.join(format!("{name}_testbin"));
        let test_dir = self.dir.join("test");
        let src_dirs = vec![self.dir.join("src")];

        // compile_test_harness needs a large stack for Clojure eval.
        let result = std::thread::Builder::new()
            .stack_size(8 * 1024 * 1024)
            .spawn({
                let test_dir = test_dir.clone();
                let bin = bin_path.clone();
                move || cljrs_compiler::aot::compile_test_harness(&test_dir, &bin, &src_dirs)
            })
            .unwrap()
            .join()
            .unwrap();
        result.unwrap_or_else(|e| panic!("test-harness compilation failed for {name}: {e:?}"));

        let output = Command::new(&bin_path)
            .output()
            .unwrap_or_else(|e| panic!("failed to run {name} test binary: {e}"));
        (
            String::from_utf8(output.stdout).unwrap(),
            output.status.success(),
        )
    }

    /// The generated (and kept) harness project directory.
    fn harness_dir(&self) -> std::path::PathBuf {
        self.dir.join(".cljrs-aot-test-harness")
    }
}

const MYLIB_CORE: &str = r#"
(ns mylib.core)

(defn add [a b]
  (+ a b))

(defn triple [x]
  (* 3 x))
"#;

#[test]
fn test_harness_passing_tests_are_natively_compiled() {
    let project = TestProject::new(
        "passing",
        &[("mylib/core.cljrs", MYLIB_CORE)],
        &[(
            "mylib/core_test.cljrs",
            r#"
(ns mylib.core-test
  (:require [clojure.test :refer [deftest is]]
            [mylib.core :as core]))

(deftest add-test
  (is (= 3 (core/add 1 2)))
  (is (= 0 (core/add -5 5))))

(deftest triple-test
  (is (= 9 (core/triple 3))))
"#,
        )],
    );

    let (stdout, success) = project.compile_and_run("passing");
    assert!(success, "test binary should exit 0\nstdout: {stdout}");
    assert!(
        stdout.contains("3 passed, 0 failed, 0 errors."),
        "unexpected summary\nstdout: {stdout}"
    );

    // The source namespace must be AOT-compiled: the generated harness links
    // and registers its native initializer instead of bundling its source.
    let main_rs = std::fs::read_to_string(project.harness_dir().join("src/main.rs")).unwrap();
    assert!(
        main_rs.contains("fn __cljrs_ns_init_"),
        "harness should declare a compiled namespace initializer:\n{main_rs}"
    );
    assert!(
        main_rs.contains("register_compiled_ns_loader(\"mylib.core\""),
        "harness should register mylib.core's compiled loader:\n{main_rs}"
    );
    assert!(
        !main_rs.contains("register_builtin_source(\"mylib.core\""),
        "mylib.core should not fall back to interpreted bundling:\n{main_rs}"
    );
    // Test namespaces are never lowered — deftests stay interpreted, and
    // macro-expanding them at compile time is the dominant cost on large
    // suites — so the test namespace must be bundled as interpreted source.
    assert!(
        main_rs.contains("register_builtin_source(\"mylib.core-test\""),
        "mylib.core-test should be bundled as interpreted source:\n{main_rs}"
    );
    assert!(
        !main_rs.contains("register_compiled_ns_loader(\"mylib.core-test\""),
        "mylib.core-test should not be lowered:\n{main_rs}"
    );
    // The object file with the compiled initializers is linked in.
    assert!(
        Path::new(&project.harness_dir().join("__cljrs_tests.o")).exists(),
        "harness should contain the compiled object file"
    );
}

#[test]
fn test_harness_failing_test_exits_nonzero() {
    let project = TestProject::new(
        "failing",
        &[("mylib/core.cljrs", MYLIB_CORE)],
        &[(
            "mylib/fail_test.cljrs",
            r#"
(ns mylib.fail-test
  (:require [clojure.test :refer [deftest is]]
            [mylib.core :as core]))

(deftest broken-test
  (is (= 999 (core/add 1 2))))
"#,
        )],
    );

    let (stdout, success) = project.compile_and_run("failing");
    assert!(
        !success,
        "test binary should exit non-zero\nstdout: {stdout}"
    );
    assert!(
        stdout.contains("0 passed, 1 failed, 0 errors."),
        "unexpected summary\nstdout: {stdout}"
    );
}
