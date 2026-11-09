//! Shared scaffolding: the compiled C fixture and an evaluation environment
//! with `clojure.rust.ffi` installed.

#![allow(dead_code)]

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

use cljrs_runtime::env::env::Env;
use cljrs_runtime::tiered::eval;
use cljrs_value::Value;

/// Build `tests/fixture/ffi_fixture.c` into a shared object, once per test
/// binary, and return its absolute path.
pub fn fixture_lib() -> &'static PathBuf {
    static LIB: OnceLock<PathBuf> = OnceLock::new();
    LIB.get_or_init(|| {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let src = manifest.join("tests/fixture/ffi_fixture.c");
        let out_dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
        let ext = if cfg!(target_os = "macos") {
            "dylib"
        } else {
            "so"
        };
        let out = out_dir.join(format!("libffi_fixture_{}.{ext}", std::process::id()));
        let cc = std::env::var("CC").unwrap_or_else(|_| "cc".to_string());
        let status = Command::new(&cc)
            .args(["-shared", "-fPIC", "-O1", "-o"])
            .arg(&out)
            .arg(&src)
            .status()
            .unwrap_or_else(|e| panic!("cannot run C compiler {cc}: {e}"));
        assert!(status.success(), "C fixture failed to compile");
        out
    })
}

/// A fresh environment with the stdlib, `clojure.data.json` and
/// `clojure.rust.ffi` installed, plus extra source paths.
pub fn env_with(source_paths: Vec<PathBuf>) -> Env {
    let globals = {
        let runtime = cljrs_runtime::Runtime::builder()
            .execution_mode(cljrs_runtime::ExecutionMode::Tiered)
            .source_paths(source_paths)
            .build()
            .expect("runtime");
        cljrs_stdlib::install(&runtime);
        runtime.into_globals()
    };
    cljrs_json::init(&globals);
    cljrs_ffi::init(&globals);
    Env::new(globals, "user")
}

pub fn try_eval(env: &mut Env, src: &str) -> Result<Value, String> {
    let mut parser = cljrs_reader::Parser::new(src.to_string(), "<ffi-test>".to_string());
    let forms = parser.parse_all().map_err(|e| format!("parse: {e:?}"))?;
    let mut out = Value::Nil;
    for form in forms {
        out = eval(&form, env).map_err(|e| format!("{e:?}"))?;
    }
    Ok(out)
}

pub fn eval_str(env: &mut Env, src: &str) -> Value {
    try_eval(env, src).unwrap_or_else(|e| panic!("eval `{src}` failed: {e}"))
}
