//! Regression tests for issue #337: shadowed `clojure.core` names must mean
//! the same thing in every tier.
//!
//! IR lowering used to decide "is this a core builtin?" from the head symbol's
//! text alone, so a `let`-bound `inc`, a parameter named `inc` or a
//! locally-`def`ed `inc` was inlined as core's — while the tree-walker called
//! the real binding.  With a low warm threshold the answer flipped mid-run, at
//! the promotion boundary; these run each program well past the threshold and
//! require every line to agree.

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Run `source` through `cljrs run` with a warm threshold of 5, so the driving
/// loop crosses into IR partway through.  Returns stdout.
fn run_tiered(source: &str) -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "cljrs_core_shadowing_{}_{nanos}_{seq}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create dir");
    let file = dir.join("shadow.cljrs");
    std::fs::write(&file, source).expect("write source");

    let output = Command::new(env!("CARGO_BIN_EXE_cljrs"))
        .arg("run")
        .arg(&file)
        .env("CLJRS_IR_THRESHOLD", "5")
        .output()
        .expect("spawn cljrs");

    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        output.status.success(),
        "cljrs exited with {:?}\nstderr:\n{}\nstdout:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );
    String::from_utf8(output.stdout).expect("utf8 stdout")
}

/// Assert every line of `stdout` is `expected` — i.e. the answer did not
/// change when the driving loop was promoted to IR.
fn assert_all_lines(stdout: &str, expected: &str, count: usize) {
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), count, "unexpected line count:\n{stdout}");
    for (i, line) in lines.iter().enumerate() {
        assert_eq!(
            *line, expected,
            "line {i} disagrees with the interpreted answer:\n{stdout}"
        );
    }
}

#[test]
fn let_bound_shadow_survives_promotion() {
    let stdout = run_tiered(
        r#"
(ns shadow.test)

(defn shadowed [n]
  (let [inc (fn [x] (* x 100))]
    (inc n)))

(defn run []
  (loop [i 0]
    (when (< i 20)
      (println (shadowed 1))
      (recur (clojure.core/inc i)))))

(run)
"#,
    );
    assert_all_lines(&stdout, "100", 20);
}

#[test]
fn parameter_shadow_survives_promotion() {
    let stdout = run_tiered(
        r#"
(ns shadow.two)

;; `inc` here is just a parameter name.
(defn apply-twice [inc n]
  (inc (inc n)))

(defn run []
  (loop [i 0]
    (when (< i 20)
      (println (apply-twice (fn [x] (* x 10)) 1))
      (recur (clojure.core/inc i)))))

(run)
"#,
    );
    assert_all_lines(&stdout, "100", 20);
}

#[test]
fn redefined_var_survives_promotion() {
    let stdout = run_tiered(
        r#"
(ns shadow.redef)

(defn inc [n] (+ n 7))

(defn run []
  (loop [i 0]
    (when (< i 20)
      (println (inc 1))
      (recur (clojure.core/inc i)))))

(run)
"#,
    );
    assert_all_lines(&stdout, "8", 20);
}

#[test]
fn refer_clojure_exclude_survives_promotion() {
    let stdout = run_tiered(
        r#"
(ns shadow.excluded
  (:refer-clojure :exclude [count]))

(defn count [coll] :not-cores)

(defn run []
  (loop [i 0]
    (when (clojure.core/< i 20)
      (println (count [1 2 3]))
      (recur (clojure.core/inc i)))))

(run)
"#,
    );
    assert_all_lines(&stdout, ":not-cores", 20);
}

#[test]
fn another_namespaces_var_is_not_core() {
    let stdout = run_tiered(
        r#"
(ns shadow.other)

(defn count [coll] :other-count)

(ns shadow.caller)

(defn run []
  (loop [i 0]
    (when (< i 20)
      (println (shadow.other/count [1 2 3]))
      (recur (inc i)))))

(run)
"#,
    );
    assert_all_lines(&stdout, ":other-count", 20);
}

#[test]
fn unshadowed_core_calls_still_work() {
    // The fix must not disturb the ordinary case: these all stay core's.
    let stdout = run_tiered(
        r#"
(ns shadow.plain)

(defn run []
  (loop [i 0]
    (when (< i 20)
      (println (inc (count [1 2])))
      (recur (inc i)))))

(run)
"#,
    );
    assert_all_lines(&stdout, "3", 20);
}
