//! `--target` is a closed set, enforced by clap rather than by a string
//! comparison in the command body.

use std::process::Command;

fn compile(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cljrs"))
        .arg("compile")
        .args(args)
        .arg("-o")
        .arg(std::env::temp_dir().join(format!("cljrs-target-{}", std::process::id())))
        .output()
        .expect("run cljrs binary")
}

#[test]
fn unknown_target_is_rejected_and_lists_the_valid_ones() {
    let out = compile(&["--target", "bogus", "app.cljrs"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "should fail: {stderr}");
    assert!(
        stderr.contains("native") && stderr.contains("wasm"),
        "{stderr}"
    );
}

#[test]
fn both_targets_are_accepted() {
    for t in ["native", "wasm"] {
        let stderr =
            String::from_utf8_lossy(&compile(&["--target", t, "no-such-file.cljrs"]).stderr)
                .into_owned();
        assert!(
            !stderr.contains("invalid value"),
            "{t} should be a valid target: {stderr}"
        );
    }
}

#[test]
fn target_defaults_to_native() {
    let stderr = String::from_utf8_lossy(&compile(&["no-such-file.cljrs"]).stderr).into_owned();
    assert!(!stderr.contains("invalid value"), "{stderr}");
}
