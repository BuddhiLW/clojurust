//! Locating the FFmpeg binaries and running them to completion.
//!
//! This crate drives the `ffmpeg` and `ffprobe` **executables** rather than
//! linking `libav*`. That is a deliberate trade:
//!
//! - the C libraries move fast and their Rust bindings lag (as of this writing
//!   `libavcodec` is at 62 and the common binding crates cap well below that),
//!   and they need `clang` + headers at build time, so a `cargo build` of this
//!   workspace would start failing on machines that merely lack a dev package;
//! - the CLI is a stable, documented interface that every platform ships, and
//!   the process boundary keeps a codec bug out of the interpreter's address
//!   space — a segfault in a filter graph kills a child, not the REPL.
//!
//! The cost is per-invocation process overhead and text parsing, which is
//! irrelevant for probing and transcoding, and avoided entirely for frame
//! streaming: `writer.rs` holds **one** long-lived child and pipes raw frames
//! into its stdin.

use std::process::{Command, Output, Stdio};

/// Path to the `ffmpeg` binary: `$CLJRS_FFMPEG`, else `ffmpeg` on `PATH`.
pub fn ffmpeg_bin() -> String {
    std::env::var("CLJRS_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_string())
}

/// Path to the `ffprobe` binary: `$CLJRS_FFPROBE`, else `ffprobe` on `PATH`.
pub fn ffprobe_bin() -> String {
    std::env::var("CLJRS_FFPROBE").unwrap_or_else(|_| "ffprobe".to_string())
}

/// Run a binary to completion, capturing both streams.
pub fn run(bin: &str, args: &[String]) -> Result<Output, String> {
    Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("cannot run `{bin}`: {e}"))
}

/// Run a binary and return its stdout, failing loudly on a non-zero exit.
///
/// FFmpeg writes its diagnostics to stderr, so the error carries stderr's tail
/// — the part that actually names what went wrong.
pub fn run_checked(bin: &str, args: &[String]) -> Result<String, String> {
    let out = run(bin, args)?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let tail: Vec<&str> = err.lines().rev().take(8).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        return Err(format!(
            "{bin} exited with {}: {}",
            out.status.code().unwrap_or(-1),
            tail.join(" | ")
        ));
    }
    String::from_utf8(out.stdout).map_err(|e| format!("{bin} produced non-UTF-8 output: {e}"))
}

/// Whether a binary is runnable at all.
pub fn available(bin: &str) -> bool {
    Command::new(bin)
        .arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
