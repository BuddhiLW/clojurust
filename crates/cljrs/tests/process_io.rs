//! `System/exit` and `read-line`, exercised as a real process: the status code
//! and the flush both only exist at the process boundary, and stdin has to be
//! written by somebody else.
use std::io::Write;
use std::process::{Command, Output, Stdio};

fn run(src: &str, input: &str) -> Output {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("process.cljrs");
    std::fs::write(&script, src).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_cljrs"))
        .arg("run")
        .arg(script)
        .current_dir(dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn system_exit_preserves_status_and_flushes_output() {
    // `print` leaves "before" in a buffer no destructor will run, and the exit
    // happens on the evaluation thread, which may be inside Tokio's scheduler:
    // both are ways to lose the status or the output.
    for status in [0, 7] {
        let out = run(
            &format!("(print \"before\") (System/exit {status}) (println \"after\")"),
            "",
        );
        assert_eq!(
            out.status.code(),
            Some(status),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(out.stdout, b"before");
        assert!(
            out.stderr.is_empty(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn read_line_handles_lines_empty_lines_and_eof() {
    let out = run(
        r#"
      (assert (= "first" (read-line)))
      (assert (= "" (read-line)))
      (assert (= "last-✓" (read-line)))
      (assert (nil? (read-line)))
      (println "stdin-ok")
    "#,
        "first\r\n\nlast-✓",
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, b"stdin-ok\n");
}
