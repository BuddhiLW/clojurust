//! CLI-level contract for `cljrs compile --require-fully-compiled`.
//!
//! Both backends audit, so both accept `--require-fully-compiled`. `--test`
//! cannot satisfy it and is refused. These tests drive the built binary via
//! `CARGO_BIN_EXE_cljrs`.

use std::path::PathBuf;
use std::process::Command;

fn compile(args: &[&str]) -> std::process::Output {
    let out: PathBuf =
        std::env::temp_dir().join(format!("cljrs-flag-combo-{}", std::process::id()));
    Command::new(env!("CARGO_BIN_EXE_cljrs"))
        .arg("compile")
        .args(args)
        .arg("-o")
        .arg(&out)
        .output()
        .expect("run cljrs binary")
}

fn stderr_of(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn require_fully_compiled_rejects_test_harness() {
    let out = compile(&["--test", "--require-fully-compiled", "."]);
    let stderr = stderr_of(&out);
    assert!(!out.status.success(), "should fail; stderr: {stderr}");
    assert!(
        stderr.contains("--require-fully-compiled") && stderr.contains("--test"),
        "message names both flags: {stderr}"
    );
}

/// wasm audits completeness now, so the flag is accepted there. It must not
/// be turned away by the combination check before the backend can apply it.
#[test]
fn require_fully_compiled_is_accepted_on_the_wasm_path() {
    let stderr = stderr_of(&compile(&[
        "--target",
        "wasm",
        "--require-fully-compiled",
        "no-such-file.cljrs",
    ]));
    assert!(
        !stderr.contains("is not supported with"),
        "wasm must not be refused for the flag: {stderr}"
    );
}

/// The combination check must not fire on the path that does audit. Compiling
/// a missing file fails for its own reason; what matters is that the failure is
/// not the combination rejection.
#[test]
fn require_fully_compiled_is_accepted_on_the_native_path() {
    let out = compile(&["--require-fully-compiled", "no-such-file.cljrs"]);
    let stderr = stderr_of(&out);
    assert!(
        !stderr.contains("is not supported with"),
        "native compile must not be rejected for the flag: {stderr}"
    );
}
