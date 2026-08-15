//! Loading native (Rust) code into a running environment.
//!
//! Two paths, both `dlopen`:
//!
//! * The **project's own** `:rust` crate from `cljrs.edn`, built by
//!   `cljrs build-native` and loaded by [`load_project_lib`] before any Clojure
//!   code runs.
//! * A **dependency's** crate at a pinned commit ([`pinned`], `:rust/load
//!   :dylib`), built on demand into a generated wrapper cdylib and loaded
//!   through an ABI handshake.
//!
//! Both are host policy, not runtime policy: `cljrs_runtime` exposes the loader
//! hooks and this is the CLI deciding to fill them in.

pub mod pinned;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cljrs_eval::GlobalEnv;

/// Return the expected on-disk path for the shared library produced by
/// `cargo build` inside `crate_dir`.
///
/// Respects cargo's workspace semantics: when `crate_dir` is a workspace
/// member, cargo writes artifacts to `<workspace_root>/target/`, not
/// `<crate_dir>/target/`. We ask cargo where its target directory is via
/// `cargo metadata`. If that fails (no cargo on PATH, malformed manifest,
/// etc.), we fall back to `<crate_dir>/target/` so the standalone-crate
/// case still works.
pub fn native_lib_path(crate_dir: &Path, crate_name: &str, release: bool) -> PathBuf {
    let profile = if release { "release" } else { "debug" };
    let lib_file = if cfg!(target_os = "windows") {
        format!("{crate_name}.dll")
    } else if cfg!(target_os = "macos") {
        format!("lib{crate_name}.dylib")
    } else {
        format!("lib{crate_name}.so")
    };
    let target_dir = cargo_target_dir(crate_dir).unwrap_or_else(|| crate_dir.join("target"));
    target_dir.join(profile).join(lib_file)
}

/// Ask `cargo metadata` for the target directory that cargo will actually use
/// when building inside `crate_dir`. Returns `None` on any failure; the caller
/// is expected to fall back to `<crate_dir>/target`.
fn cargo_target_dir(crate_dir: &Path) -> Option<PathBuf> {
    let output = std::process::Command::new("cargo")
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--offline",
        ])
        .current_dir(crate_dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = std::str::from_utf8(&output.stdout).ok()?;
    // Cargo's metadata JSON puts `target_directory` at the top level. We do
    // a small targeted extract rather than pulling in a JSON dependency.
    let key = r#""target_directory":""#;
    let start = stdout.find(key)? + key.len();
    let rest = &stdout[start..];
    let mut end = None;
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if i + 1 < bytes.len() => i += 2,
            b'"' => {
                end = Some(i);
                break;
            }
            _ => i += 1,
        }
    }
    let raw = &rest[..end?];
    Some(PathBuf::from(json_unescape(raw)))
}

/// Decode the small subset of JSON string escapes that can appear in a
/// `target_directory` path emitted by `cargo metadata`.
fn json_unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some('/') => out.push('/'),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Load the shared library declared by the project's `:rust` config and call
/// its `cljrs_init`
/// entry point to register native functions into `globals`.
///
/// A missing library emits a warning and returns — callers of unregistered
/// functions will get a runtime error rather than a startup crash, which is
/// friendlier during development.
pub fn load_project_lib(rust_config: &cljrs_project::config::RustConfig, globals: &Arc<GlobalEnv>) {
    let Some(init_fn) = rust_config.init_fn.as_deref() else {
        return;
    };
    let Some(crate_name) = rust_config.crate_name() else {
        return;
    };
    // Symbol name is the last segment of the Rust path, e.g. "cljrs_init".
    let sym_name = init_fn.rsplit("::").next().unwrap_or(init_fn);

    let lib_path = native_lib_path(&rust_config.crate_dir, crate_name, false);
    if !lib_path.exists() {
        eprintln!(
            "cljrs: native library not found at {} — run `cljrs build-native` first",
            lib_path.display()
        );
        return;
    }

    // SAFETY: we own the process and are responsible for ensuring the library
    // stays loaded (via mem::forget below) for the entire lifetime of globals.
    unsafe {
        let lib = match libloading::Library::new(&lib_path) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("cljrs: could not load {}: {e}", lib_path.display());
                return;
            }
        };

        // The exported symbol has C linkage and takes a raw pointer so it is
        // callable across the FFI boundary without ABI assumptions.
        let sym_bytes: Vec<u8> = format!("{sym_name}\0").into_bytes();
        let init: libloading::Symbol<unsafe extern "C" fn(*mut cljrs_interop::Registry)> =
            match lib.get(&sym_bytes) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!(
                        "cljrs: could not find symbol {sym_name} in {}: {e}",
                        lib_path.display()
                    );
                    return;
                }
            };

        let mut registry = cljrs_interop::Registry::new(globals.clone());
        init(&mut registry as *mut _);

        // Prevent the library from being unloaded — its code must remain
        // reachable as long as any registered NativeFn closures exist.
        std::mem::forget(lib);
    }
    eprintln!(
        "[build-native] loaded {} ({})",
        lib_path.display(),
        sym_name
    );
}
