//! Runs the Clojure-side `clojure.test` suites for `cljrs.ffmpeg`.
//!
//! Two namespaces, deliberately split:
//!
//! - `cljrs.ffmpeg-test` — argument validation and refusal behaviour. Runs
//!   everywhere; starts no process.
//! - `cljrs.ffmpeg-encode-test` — actually encodes, probes and extracts. Runs
//!   only when `ffmpeg` and `ffprobe` are on the PATH, and is **skipped
//!   loudly** otherwise. Reporting a pass for a suite that never ran would make
//!   a green CI on a machine without FFmpeg mean nothing.
//!
//! The encoding suite writes files with relative names, so this harness moves
//! the process into a fresh temporary directory first. That is process-global
//! state; it is safe here because this binary runs exactly one test.

use std::path::PathBuf;

use cljrs_interop::Registry;
use cljrs_runtime::tiered::{Env, eval};
use cljrs_value::Value;

const PURE_NSES: &[&str] = &["cljrs.ffmpeg-test"];
const ENCODE_NSES: &[&str] = &["cljrs.ffmpeg-encode-test"];

fn parse_one(src: &str) -> cljrs_reader::Form {
    let mut parser = cljrs_reader::Parser::new(src.to_string(), "<test-driver>".to_string());
    let forms = parser.parse_all().expect("test driver: parse failed");
    forms
        .into_iter()
        .next()
        .expect("test driver: expected at least one form")
}

fn eval_str(env: &mut Env, src: &str) -> Value {
    let form = parse_one(src);
    eval(&form, env).unwrap_or_else(|e| panic!("eval `{src}` failed: {e:?}"))
}

fn extract_counter(map: &Value, key: &str) -> i64 {
    let mut found = 0i64;
    if let Value::Map(m) = map {
        m.for_each(|k, v| {
            if let (Value::Keyword(kw), Value::Long(n)) = (k, v)
                && kw.get().name.as_ref() == key
            {
                found = *n;
            }
        });
    }
    found
}

#[test]
fn run_clojure_ffmpeg_tests() {
    let test_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test");
    assert!(
        test_dir.is_dir(),
        "test dir not found: {}",
        test_dir.display()
    );

    let have_ffmpeg = cljrs_ffmpeg::proc::available(&cljrs_ffmpeg::proc::ffmpeg_bin())
        && cljrs_ffmpeg::proc::available(&cljrs_ffmpeg::proc::ffprobe_bin());

    // The encoding suite writes relative paths; give it somewhere to write.
    let workdir = tempfile::tempdir().expect("tempdir");
    std::env::set_current_dir(workdir.path()).expect("chdir into the temp dir");

    let _mutator = cljrs_gc::register_mutator();

    let globals = {
        let runtime = cljrs_runtime::Runtime::builder()
            .execution_mode(cljrs_runtime::ExecutionMode::Tiered)
            .source_paths(vec![test_dir])
            .build()
            .expect("runtime");
        cljrs_stdlib::install(&runtime);
        runtime.into_globals()
    };

    let mut registry = Registry::new(globals.clone());
    cljrs_ffmpeg::register(&mut registry);
    // Deliberately NOT registering `cljrs.raster` here: this crate does not
    // depend on it (two `cljrs_init` symbols cannot share a link), so the
    // encoding suite builds its frames out of plain bytes. The canvas-to-video
    // pipeline is exercised where both namespaces really do coexist — the
    // `cljrs` binary — by `samples/raster_video.cljrs`.

    let mut env = Env::new(globals, "user");
    cljrs_runtime::env::callback::push_eval_context(&env);

    eval_str(&mut env, "(require 'clojure.test)");

    let mut nses: Vec<&str> = PURE_NSES.to_vec();
    if have_ffmpeg {
        nses.extend_from_slice(ENCODE_NSES);
    } else {
        eprintln!(
            "[ffmpeg-tests] SKIPPING {} — ffmpeg/ffprobe not found on PATH \
             (set $CLJRS_FFMPEG / $CLJRS_FFPROBE to point at them)",
            ENCODE_NSES.join(", ")
        );
    }

    let mut total_pass = 0i64;
    let mut total_fail = 0i64;
    let mut total_error = 0i64;
    let mut total_test = 0i64;
    let mut failures: Vec<String> = Vec::new();

    for ns in &nses {
        eprintln!("[ffmpeg-tests] running {ns}");
        eval_str(&mut env, &format!("(require '{ns})"));
        let result = eval_str(&mut env, &format!("(clojure.test/run-tests '{ns})"));
        let pass = extract_counter(&result, "pass");
        let fail = extract_counter(&result, "fail");
        let error = extract_counter(&result, "error");
        let test = extract_counter(&result, "test");
        total_pass += pass;
        total_fail += fail;
        total_error += error;
        total_test += test;
        if fail > 0 || error > 0 {
            failures.push(format!("{ns}: {fail} fail, {error} error"));
        }
    }

    cljrs_runtime::env::callback::pop_eval_context();

    eprintln!(
        "[ffmpeg-tests] totals: {total_test} tests, {total_pass} pass, \
         {total_fail} fail, {total_error} error (encoding suite {})",
        if have_ffmpeg { "ran" } else { "SKIPPED" },
    );

    assert!(total_test > 0, "no tests ran");
    assert!(
        failures.is_empty(),
        "Clojure-side test failures:\n  {}",
        failures.join("\n  "),
    );
}
