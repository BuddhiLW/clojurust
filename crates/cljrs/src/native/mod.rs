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

use cljrs_runtime::tiered::GlobalEnv;

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
    target_dir_from_metadata(std::str::from_utf8(&output.stdout).ok()?)
}

/// Pull `target_directory` out of `cargo metadata --format-version 1` output.
///
/// Split from [`cargo_target_dir`] so it can be tested without a subprocess.
/// This parses the document rather than scanning for the key: a path can carry
/// JSON escapes (`\\` on Windows, `\"` in a pathological directory name), and
/// nothing in cargo's contract promises the compact, unordered formatting a
/// scanner would depend on.
fn target_dir_from_metadata(json: &str) -> Option<PathBuf> {
    let meta: serde_json::Value = serde_json::from_str(json).ok()?;
    Some(PathBuf::from(meta.get("target_directory")?.as_str()?))
}

/// Load the shared library declared by the project's `:rust` config and call
/// its `cljrs_init_<crate>`
/// entry point to register native functions into `globals`.
///
/// A missing library emits a warning and returns — callers of unregistered
/// functions will get a runtime error rather than a startup crash, which is
/// friendlier during development.
/// An ABI mismatch instead returns an error: startup must stop before calling init.
pub fn load_project_lib(
    rust_config: &cljrs_project::config::RustConfig,
    globals: &Arc<GlobalEnv>,
) -> Result<(), String> {
    let Some(init_fn) = rust_config.init_fn.as_deref() else {
        return Ok(());
    };
    let Some(crate_name) = rust_config.crate_name() else {
        return Ok(());
    };
    // Symbol name is the last segment of the Rust path, e.g. "cljrs_init_my_project".
    let sym_name = init_fn.rsplit("::").next().unwrap_or(init_fn);

    let lib_path = native_lib_path(&rust_config.crate_dir, crate_name, false);
    if !lib_path.exists() {
        eprintln!(
            "cljrs: native library not found at {} — run `cljrs build-native` first",
            lib_path.display()
        );
        return Ok(());
    }

    // SAFETY: we own the process and are responsible for ensuring the library
    // stays loaded (via mem::forget below) for the entire lifetime of globals.
    unsafe {
        let lib = match libloading::Library::new(&lib_path) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("cljrs: could not load {}: {e}", lib_path.display());
                return Ok(());
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
                    return Ok(());
                }
            };

        check_project_abi(
            &lib,
            std::env::var("CLJRS_NATIVE_STRICT").as_deref() == Ok("1"),
        )?;

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
    Ok(())
}

/// Verify a project's Rust ABI before touching its Registry pointer.
/// Legacy crates are accepted only when strict mode is disabled.
unsafe fn check_project_abi(lib: &libloading::Library, strict: bool) -> Result<(), String> {
    let abi: libloading::Symbol<unsafe extern "C" fn() -> *const std::os::raw::c_char> =
        match unsafe { lib.get(pinned::ABI_SYMBOL) } {
            Ok(abi) => abi,
            Err(_) if strict => return Err("native library has no cljrs_dylib_abi symbol; CLJRS_NATIVE_STRICT=1 refuses unverified Rust ABI; rebuild with `cljrs build-native`".into()),
            Err(_) => {
                eprintln!("cljrs: warning: native library has no cljrs_dylib_abi symbol; unverified Rust ABI may crash; rebuild with `cljrs build-native`");
                return Ok(());
            }
        };
    let ptr = unsafe { abi() };
    if ptr.is_null() {
        return Err(
            "native library returned a null ABI fingerprint; rebuild with `cljrs build-native`"
                .into(),
        );
    }
    let got = unsafe { std::ffi::CStr::from_ptr(ptr) }.to_string_lossy();
    let expected = pinned::abi_fingerprint();
    if got != expected {
        return Err(format!(
            "native ABI fingerprint mismatch: library was built as `{got}` but this binary expects `{expected}`; rebuild with `cljrs build-native`"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Compile real cdylibs to exercise symbol resolution, not just a string comparator.
    #[cfg(unix)]
    fn fixture(cfg: &str) -> (tempfile::TempDir, libloading::Library) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("libnative_abi_fixture.so");
        let mut cmd = std::process::Command::new("rustc");
        cmd.args(["--crate-type", "cdylib", "--edition", "2024"])
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/native_abi.rs"
            ))
            .args(["-o"])
            .arg(&path)
            .env("CLJRS_TEST_ABI", pinned::abi_fingerprint());
        if !cfg.is_empty() {
            cmd.args(["--cfg", cfg]);
        }
        assert!(cmd.status().unwrap().success());
        let lib = unsafe { libloading::Library::new(path).unwrap() };
        (dir, lib)
    }

    #[cfg(unix)]
    #[test]
    fn matching_fingerprint_allows_init() {
        let (_dir, lib) = fixture("matching");
        unsafe {
            check_project_abi(&lib, false).unwrap();
            let init = lib
                .get::<unsafe extern "C" fn(*mut ())>(b"cljrs_init_abi_fixture\0")
                .unwrap();
            let count = lib
                .get::<unsafe extern "C" fn() -> usize>(b"test_init_calls\0")
                .unwrap();
            init(std::ptr::null_mut());
            assert_eq!(count(), 1);
        }
    }

    #[cfg(unix)]
    #[test]
    fn mismatched_fingerprint_refuses_without_calling_init() {
        let (_dir, lib) = fixture("");
        unsafe {
            let err = check_project_abi(&lib, false).unwrap_err();
            assert!(
                err.contains("cljrs incompatible; rustc other; debug"),
                "{err}"
            );
            assert!(err.contains(&pinned::abi_fingerprint()), "{err}");
            assert!(err.contains("rebuild with `cljrs build-native`"), "{err}");
            let count = lib
                .get::<unsafe extern "C" fn() -> usize>(b"test_init_calls\0")
                .unwrap();
            assert_eq!(count(), 0);
        }
    }

    #[cfg(unix)]
    #[test]
    fn legacy_fingerprint_warns_and_strict_refuses() {
        let (_dir, lib) = fixture("legacy");
        unsafe {
            check_project_abi(&lib, false).unwrap();
            let err = check_project_abi(&lib, true).unwrap_err();
            assert!(err.contains("CLJRS_NATIVE_STRICT=1"), "{err}");
            let count = lib
                .get::<unsafe extern "C" fn() -> usize>(b"test_init_calls\0")
                .unwrap();
            assert_eq!(count(), 0);
            let init = lib
                .get::<unsafe extern "C" fn(*mut ())>(b"cljrs_init_abi_fixture\0")
                .unwrap();
            init(std::ptr::null_mut());
            assert_eq!(count(), 1);
        }
    }

    /// The shape cargo emits today: one compact line.
    #[test]
    fn reads_target_directory_from_compact_metadata() {
        let json =
            r#"{"packages":[],"workspace_root":"/w","target_directory":"/w/target","version":1}"#;
        assert_eq!(
            target_dir_from_metadata(json),
            Some(PathBuf::from("/w/target"))
        );
    }

    /// Nothing promises compact output, and a key-scanner would miss the space
    /// after the colon. Ordering is not load-bearing either.
    #[test]
    fn reads_target_directory_from_pretty_printed_metadata() {
        let json = r#"{
  "version": 1,
  "target_directory": "/w/target",
  "workspace_root": "/w"
}"#;
        assert_eq!(
            target_dir_from_metadata(json),
            Some(PathBuf::from("/w/target"))
        );
    }

    /// Windows paths arrive with every separator escaped.
    #[test]
    fn unescapes_a_windows_path() {
        let json = r#"{"target_directory":"C:\\Users\\dev\\my crate\\target"}"#;
        assert_eq!(
            target_dir_from_metadata(json),
            Some(PathBuf::from(r"C:\Users\dev\my crate\target"))
        );
    }

    /// A directory name may legally contain a quote or a backslash on unix.
    #[test]
    fn unescapes_quotes_and_backslashes() {
        let json = r#"{"target_directory":"/w/a\"b\\c/target"}"#;
        assert_eq!(
            target_dir_from_metadata(json),
            Some(PathBuf::from("/w/a\"b\\c/target"))
        );
    }

    /// Every failure is `None`, so `native_lib_path` falls back to
    /// `<crate_dir>/target` rather than producing a wrong path.
    #[test]
    fn absent_or_unusable_metadata_is_none() {
        assert_eq!(target_dir_from_metadata(r#"{"version":1}"#), None);
        assert_eq!(target_dir_from_metadata(r#"{"target_directory":7}"#), None);
        assert_eq!(target_dir_from_metadata("not json at all"), None);
        assert_eq!(target_dir_from_metadata(""), None);
    }

    /// The fallback path is joined from the metadata answer, not the crate dir.
    #[test]
    fn native_lib_path_falls_back_to_crate_local_target() {
        // A directory with no manifest makes `cargo metadata` fail, so this
        // exercises the `unwrap_or_else` fallback.
        let dir = std::env::temp_dir().join("cljrs-no-manifest-here");
        let path = native_lib_path(&dir, "mylib", false);
        assert!(path.starts_with(&dir), "{}", path.display());
        assert!(path.to_string_lossy().contains("debug"));
    }
}
