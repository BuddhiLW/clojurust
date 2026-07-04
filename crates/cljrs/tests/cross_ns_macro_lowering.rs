//! Regression test for issue #262.
//!
//! `macros::resolve_macro` (the macro lookup used by the background IR
//! lowering path) resolved a namespace-qualified head symbol by treating the
//! namespace segment as a literal namespace name, instead of resolving it
//! through the caller's `:require :as` alias table the way ordinary symbol
//! evaluation (`eval_symbol`) does. A macro invoked through its require
//! alias (`m/m`) was therefore invisible to `macroexpand_all`: the call was
//! shipped to the IR lowerer unexpanded, which lowered it as a call to the
//! macro's own function value — not callable at runtime.
//!
//! The bug only manifests once a function crosses the background-lowering
//! warm threshold (`ir_threshold`, default 50), so this drives the call
//! count well past that with a low threshold to make it deterministic.

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn cross_namespace_macro_survives_background_lowering() {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "cljrs_cross_ns_macro_{}_{nanos}_{seq}",
        std::process::id()
    ));
    let ns_dir = dir.join("repro");
    std::fs::create_dir_all(&ns_dir).expect("create ns dir");

    std::fs::write(
        ns_dir.join("macro_ns.cljrs"),
        r#"
(ns repro.macro-ns)
(defmacro m [x] `(inc ~x))
"#,
    )
    .expect("write macro_ns");

    std::fs::write(
        ns_dir.join("caller.cljrs"),
        r#"
(ns repro.caller (:require [repro.macro-ns :as m]))
(defn get-it [x] (m/m x))
(dotimes [i 60] (println (get-it i)))
(println "60 calls succeeded, no error")
"#,
    )
    .expect("write caller");

    let output = Command::new(env!("CARGO_BIN_EXE_cljrs"))
        .arg("run")
        .arg(ns_dir.join("caller.cljrs"))
        .arg("--src-path")
        .arg(&dir)
        .env("CLJRS_IR_THRESHOLD", "10")
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

    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    let expected: Vec<String> = (0..60).map(|i| (i + 1).to_string()).collect();
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines.len(),
        61,
        "expected 60 results + summary line:\n{stdout}"
    );
    assert_eq!(
        lines[..60],
        expected,
        "cross-namespace macro call produced wrong values after background lowering:\n{stdout}"
    );
    assert_eq!(lines[60], "60 calls succeeded, no error");
}
