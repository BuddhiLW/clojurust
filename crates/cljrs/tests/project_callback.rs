//! Real project cdylib boundary: the plugin has its own runtime code and TLS.
use std::path::Path;
use std::process::Command;

#[test]
#[ignore = "builds a project cdylib; run explicitly with --ignored"]
fn project_callback_grows_vectors_past_a_leaf() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let project = tempfile::tempdir().unwrap();
    let dir = project.path();
    std::fs::create_dir(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        format!(
            r#"
[package]
name = "callback-probe"
version = "0.1.0"
edition = "2024"
[workspace]
[lib]
crate-type = ["cdylib"]
[dependencies]
archery = "=1.2.3"
cljrs-interop = {{ path = "{}/crates/cljrs-interop" }}
cljrs-runtime = {{ path = "{}/crates/cljrs-runtime" }}
cljrs-value = {{ path = "{}/crates/cljrs-value" }}
"#,
            root.display(),
            root.display(),
            root.display()
        ),
    )
    .unwrap();
    std::fs::copy(root.join("Cargo.lock"), dir.join("Cargo.lock")).unwrap();
    std::fs::write(
        dir.join("src/lib.rs"),
        r#"
use cljrs_interop::Registry;
use cljrs_runtime::env::callback;
use cljrs_value::{Arity, NativeFn};
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cljrs_init(registry: *mut Registry) {
    let registry = unsafe { &mut *registry };
    callback::install_eval_context(registry.env().clone(), "user".into());
    registry.define("probe/invoke", NativeFn::new("invoke", Arity::Fixed(2), |args| {
        callback::invoke(&args[0], vec![args[1].clone()])
    }));
}
"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("cljrs.edn"),
        r#"{:paths ["src"] :rust {:crate "." :init "callback_probe::cljrs_init"}}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("probe.cljrs"),
        r#"
      (defn grow [n] (loop [i 0 acc []] (if (< i n) (recur (inc i) (conj acc i)) acc)))
      (doseq [n [0 31 32 33 40 1025]]
        (let [direct (grow n) via (probe/invoke grow n)]
          (assert (= n (count via)))
          (assert (= direct via))))
      (assert (= 42 (probe/invoke {:answer 42} :answer)))
      (println "callback-ok")
"#,
    )
    .unwrap();
    // Reuse the host's build cache but keep fixture source and lock isolated.
    let target = root.join("target");
    let build = Command::new("cargo")
        .args(["build", "--offline", "-j", "1"])
        .current_dir(dir)
        .env("CARGO_TARGET_DIR", &target)
        .env("CARGO_BUILD_JOBS", "1")
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let output = Command::new(env!("CARGO_BIN_EXE_cljrs"))
        .args(["run", "probe.cljrs"])
        .current_dir(dir)
        .env("CARGO_TARGET_DIR", &target)
        .env("CARGO_BUILD_JOBS", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("callback-ok"));
}
