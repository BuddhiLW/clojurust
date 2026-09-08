//! Native packages: build a dependency's Rust crate as a cdylib and load it
//! (`:rust/load :dylib` in `cljrs.edn`).
//!
//! Two dependency forms reach this module, and they differ only in where the
//! crate's source comes from:
//!
//! - `:git/url` + `:git/sha` — an immutable commit. Serves both `require` and
//!   versioned (`mylib/f@<sha>`) resolution; its artifact is cached forever.
//! - `:local/root` — a working tree, built as it currently stands. Serves
//!   `require` only (there is no commit to resolve `@<sha>` against), and is
//!   rebuilt on every load because the tree changes underfoot.
//!
//! By default, a pinned symbol (`mylib/f@<sha>`) that resolves to a native
//! (Rust-backed) function falls back to the **current binary's**
//! implementation, with provenance verification (see
//! `cljrs_runtime::env::versioned`).  This module provides the opt-in
//! alternative: true out-of-binary native code.
//!
//! ## Flow
//!
//! 1. `install(globals)` registers the loader hooks on the environment.  The
//!    versioned resolver calls one whenever a pinned lookup is about to fall
//!    back to a native function; `require` calls the other for a namespace no
//!    source path provides.
//! 2. The hook checks `cljrs.edn` for a dep covering the namespace with
//!    `:rust/load :dylib` and a `:rust/init` function.
//! 3. The dep's source is materialized — fetched and checked out at the
//!    pinned commit (`cljrs_project::vcs`), or taken as-is from
//!    `:local/root` — and wrapped in a generated cdylib crate that pins the
//!    exact same `cljrs-interop` as the host.
//! 4. `cargo build` in the host's profile (cached per `(crate, source, rustc,
//!    cljrs version)`), then `dlopen`.
//! 5. **ABI handshake**: the wrapper exports `cljrs_dylib_abi()` returning
//!    a fingerprint string (cljrs version + `rustc -V`, baked at the
//!    wrapper's build time).  The host refuses to proceed unless it equals
//!    the host's own fingerprint exactly.
//! 6. The wrapper's `cljrs_dylib_init(*mut Registry)` registers the
//!    package's exports through a [`Registry::versioned`] view, so the
//!    pinned implementations land in the immutable `"<ns>@<commit>"`
//!    namespace and never collide with the live ones.
//!
//! ## Safety model (experimental)
//!
//! `cljrs_dylib_init` crosses the boundary with a Rust-ABI `&mut Registry`.
//! This is sound *only* because the handshake guarantees both sides were
//! compiled by the identical compiler against the identical `cljrs-interop`
//! version.  Feature-flag skew between host and wrapper builds is not
//! detected; the whole mechanism is opt-in and documented as experimental.
//! A full C-ABI vtable is the safer long-term design and is deliberately
//! deferred.

// EvalError is large by design across the workspace (same allow as cljrs-runtime).
#![allow(clippy::result_large_err)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cljrs_project::config::Dependency;
use cljrs_runtime::env::error::{EvalError, EvalResult};
use cljrs_runtime::tiered::GlobalEnv;

/// Exported ABI-handshake symbol name.
pub const ABI_SYMBOL: &[u8] = b"cljrs_dylib_abi\0";
/// Exported init symbol name.
pub const INIT_SYMBOL: &[u8] = b"cljrs_dylib_init\0";

/// The host's ABI fingerprint: cljrs workspace version, the rustc that
/// compiled this crate, and the build profile (debug/release — `cljrs-gc`'s
/// object headers have `debug_assertions`-gated fields, so the profiles must
/// match).  A wrapper dylib is only loaded when its baked fingerprint equals
/// this string exactly.
pub fn abi_fingerprint() -> String {
    format!(
        "cljrs {}; {}; {}",
        env!("CARGO_PKG_VERSION"),
        env!("CLJRS_DYLIB_RUSTC"),
        host_profile(),
    )
}

/// The host's build profile, which the wrapper build must match.
fn host_profile() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

/// Install the pinned-native package loader on `globals`.
///
/// Idempotent (first writer wins).  Called by [`crate::session::setup_globals`]
/// during environment setup.
pub fn install(globals: &Arc<GlobalEnv>) {
    globals.set_pinned_native_loader(Arc::new(load_pinned));
    globals.set_native_require_loader(Arc::new(load_require));
}

/// The loader hook: returns `Ok(true)` when the package covering `base_ns`
/// was built at `commit` and registered into `"<base_ns>@<commit>"`.
fn load_pinned(globals: &Arc<GlobalEnv>, base_ns: &str, commit: &str) -> EvalResult<bool> {
    let config = globals.deps_config.read().unwrap().clone();
    let Some(config) = config else {
        return Ok(false);
    };
    let Some(dep) = find_dylib_dep(&config, base_ns) else {
        return Ok(false);
    };
    let dep = dep.map_err(|e| EvalError::Runtime(format!("pinned native {base_ns}: {e}")))?;
    // A working tree has no commit, so it cannot answer for `ns/f@sha`.
    // Decline and let the versioned resolver fall back as it would have.
    if !dep.is_pinned() {
        return Ok(false);
    }

    let versioned_ns = format!("{base_ns}@{commit}");
    if globals.is_loaded(&versioned_ns) {
        return Ok(true);
    }

    let lib_path = build_wrapper(&dep, Some(commit))
        .map_err(|e| EvalError::Runtime(format!("pinned native {versioned_ns}: {e}")))?;

    load_library(globals, &lib_path, Some(commit))
        .map_err(|e| EvalError::Runtime(format!("pinned native {versioned_ns}: {e}")))?;

    globals.mark_loaded(&versioned_ns);
    eprintln!("[cljrs] loaded pinned native package {versioned_ns}");
    Ok(true)
}

/// The `require`-path loader hook: returns `Ok(true)` when a `:rust/load
/// :dylib` dep covering `ns` was built and its exports were registered into
/// the **unversioned** namespace, making a plain `(require '[ns :as …])` of a
/// pure-native package succeed.
///
/// Unlike [`load_pinned`] (which serves versioned-symbol resolution and lands
/// the package in the immutable `"<ns>@<commit>"` namespace), this registers
/// into the live namespace so unversioned references resolve normally.  The
/// caller (`cljrs-runtime`'s unversioned loader) marks `ns` loaded on success.
///
/// Both dependency forms reach this path: a git dep builds at its pinned
/// `:git/sha`, a `:local/root` dep builds the working tree as it stands.
fn load_require(globals: &Arc<GlobalEnv>, ns: &str) -> EvalResult<bool> {
    let config = globals.deps_config.read().unwrap().clone();
    let Some(config) = config else {
        return Ok(false);
    };
    let Some(dep) = find_dylib_dep(&config, ns) else {
        return Ok(false);
    };
    let dep = dep.map_err(|e| EvalError::Runtime(format!("native dep {ns}: {e}")))?;

    // Already brought in (e.g. an earlier require of a sibling namespace
    // provided by the same dylib loaded the whole package).
    if globals.is_loaded(ns) {
        return Ok(true);
    }

    let lib_path = build_wrapper(&dep, None)
        .map_err(|e| EvalError::Runtime(format!("native dep {ns}: {e}")))?;

    load_library(globals, &lib_path, None)
        .map_err(|e| EvalError::Runtime(format!("native dep {ns}: {e}")))?;

    eprintln!("[cljrs] loaded native dep {ns} ({})", dep.provenance());
    Ok(true)
}

// ── Domain: what a native dependency IS ───────────────────────────────────────

/// Where a `:rust/load :dylib` dep's crate source comes from.
///
/// A closed set: every source a `cljrs.edn` dep can name is one of these.
#[derive(Debug, Clone)]
enum NativeSource {
    /// A git repository, checked out at a pinned commit.
    Pinned { url: Arc<str>, sha: Arc<str> },
    /// A directory on disk, built as it currently stands.
    WorkingTree(PathBuf),
}

impl NativeSource {
    /// The commit this source is fixed at, or `None` for a working tree.
    fn commit(&self) -> Option<&str> {
        match self {
            NativeSource::Pinned { sha, .. } => Some(sha.as_ref()),
            NativeSource::WorkingTree(_) => None,
        }
    }
}

/// What exactly was built — the identity that names one artifact.
///
/// A commit names it for a git dep; a working tree has no commit, so its
/// content does. Either way an artifact under this version is immutable, which
/// is what lets the cache be trusted and what keeps a rebuilt library from
/// landing on the path an already-`dlopen`ed one occupies.
struct SourceVersion {
    /// The cache-directory suffix, e.g. `@<sha>` or `@local-<hash>`.
    slug: String,
    /// The full key mixed into the ABI-fingerprint hash.
    key: String,
}

/// A dependency that opts into loading its Rust code as a cdylib.
///
/// Constructing one is the validation step: a `NativeDep` always names an
/// init function, so no later stage has to handle its absence.
#[derive(Debug, Clone)]
struct NativeDep {
    source: NativeSource,
    init_fn: Arc<str>,
    crate_subdir: Option<Arc<str>>,
}

impl NativeDep {
    /// Build a dep from a declaration, or `Err` when it opted into `:dylib`
    /// without naming the `:rust/init` function that would load it.
    fn new(
        source: NativeSource,
        rust_init: Option<Arc<str>>,
        crate_subdir: Option<Arc<str>>,
    ) -> Result<Self, String> {
        let init_fn = rust_init.ok_or("dep has :rust/load :dylib but no :rust/init function")?;
        Ok(NativeDep {
            source,
            init_fn,
            crate_subdir,
        })
    }

    /// Whether this dep names an immutable commit, and so can serve versioned
    /// (`ns/f@sha`) resolution.
    fn is_pinned(&self) -> bool {
        self.source.commit().is_some()
    }

    /// The Cargo package name, as the init path's first segment spells it.
    fn crate_name(&self) -> &str {
        self.init_fn.split("::").next().unwrap_or(&self.init_fn)
    }

    /// The `extern crate` identifier for [`Self::crate_name`].
    fn pkg_ident(&self) -> String {
        self.crate_name().replace('-', "_")
    }

    /// The init path with its crate segment stripped (`a::b::c` -> `b::c`).
    fn init_tail(&self) -> &str {
        self.init_fn
            .split_once("::")
            .map(|(_, rest)| rest)
            .unwrap_or(&self.init_fn)
    }

    /// Human-readable provenance, for the load log line.
    fn provenance(&self) -> String {
        match &self.source {
            NativeSource::Pinned { sha, .. } => format!("pinned {sha}"),
            NativeSource::WorkingTree(root) => format!("local {}", root.display()),
        }
    }
}

/// Read the `:rust/*` keys of a `:deps` entry as a native dep, or `None` when
/// the entry did not opt into `:rust/load :dylib`.
fn as_native_dep(dep: &Dependency) -> Option<Result<NativeDep, String>> {
    match dep {
        Dependency::Git(git) if git.rust_load_dylib => Some(NativeDep::new(
            NativeSource::Pinned {
                url: git.url.clone(),
                sha: git.sha.clone(),
            },
            git.rust_init.clone(),
            git.rust_crate_dir.clone(),
        )),
        Dependency::Local {
            root,
            rust_init,
            rust_crate_dir,
            rust_load_dylib: true,
        } => Some(NativeDep::new(
            NativeSource::WorkingTree(root.clone()),
            rust_init.clone(),
            rust_crate_dir.clone(),
        )),
        _ => None,
    }
}

/// Whether a dep named `dep_name` provides namespace `ns` — exact match, or a
/// dotted prefix (dep `my.lib` covers `my.lib.util`).
fn covers_namespace(dep_name: &str, ns: &str) -> bool {
    dep_name == ns
        || ns
            .strip_prefix(dep_name)
            .is_some_and(|rest| rest.starts_with('.'))
}

/// Find the `:rust/load :dylib` dep covering `base_ns`.
fn find_dylib_dep(
    config: &cljrs_project::config::DepsConfig,
    base_ns: &str,
) -> Option<Result<NativeDep, String>> {
    config
        .deps
        .iter()
        .filter(|(name, _)| covers_namespace(name, base_ns))
        .find_map(|(_, dep)| as_native_dep(dep))
}

// ── Wrapper build ─────────────────────────────────────────────────────────────

/// Where the wrapper for one `(dep, version)` is generated and what it produces.
struct WrapperPlan {
    /// The dep's own crate, already on disk.
    crate_dir: PathBuf,
    /// The generated wrapper crate's directory.
    wrapper_dir: PathBuf,
    /// The cdylib `cargo` will produce inside `wrapper_dir`.
    artifact: PathBuf,
    /// Cache identity, for log lines.
    label: String,
}

/// Build (or reuse from cache) the wrapper cdylib for `dep`, returning the
/// path to the built library.
///
/// `commit_override` builds a pinned dep at a commit other than its declared
/// `:git/sha` — the versioned resolver asks for the commit named in the symbol.
/// It must be `None` for a working-tree dep, which has no commit at all.
fn build_wrapper(dep: &NativeDep, commit_override: Option<&str>) -> Result<PathBuf, String> {
    let source_root = materialize_source(&dep.source, commit_override)?;
    let crate_dir = crate_dir_of(dep, &source_root);
    if !crate_dir.join("Cargo.toml").exists() {
        return Err(format!(
            "no Cargo.toml at {} (set :rust/crate if the crate lives in a subdirectory)",
            crate_dir.display()
        ));
    }

    let version = resolve_version(&dep.source, &crate_dir, commit_override)?;
    let plan = plan_wrapper(dep, crate_dir, &version);
    if plan.artifact.exists() {
        return Ok(plan.artifact);
    }

    write_wrapper_crate(&plan.wrapper_dir, &plan.crate_dir, dep)?;
    cargo_build(&plan)?;

    if !plan.artifact.exists() {
        return Err(format!(
            "built wrapper not found at {}",
            plan.artifact.display()
        ));
    }
    Ok(plan.artifact)
}

/// Identify what is about to be built.
///
/// A pinned dep is named by its commit. A working tree is named by a digest of
/// its source files, so an edited tree is a different version — which is what
/// gets it rebuilt, and gets the result its own path to be loaded from.
fn resolve_version(
    source: &NativeSource,
    crate_dir: &Path,
    commit_override: Option<&str>,
) -> Result<SourceVersion, String> {
    match source {
        NativeSource::Pinned { url, sha } => {
            let commit = commit_override.unwrap_or(sha.as_ref());
            Ok(SourceVersion {
                slug: format!("@{commit}"),
                key: format!("{url}|{commit}"),
            })
        }
        NativeSource::WorkingTree(root) => {
            let digest = digest_source_tree(crate_dir)?;
            Ok(SourceVersion {
                slug: format!("@local-{digest}"),
                key: format!("{}|{digest}", root.display()),
            })
        }
    }
}

/// Digest every source file under `dir`, ignoring build output and VCS data.
///
/// Path and contents both feed the hash, so a rename is as much a change as an
/// edit. Walk order is sorted, making the digest independent of readdir order.
fn digest_source_tree(dir: &Path) -> Result<String, String> {
    let mut acc = String::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let mut entries: Vec<_> = std::fs::read_dir(&current)
            .map_err(|e| format!("reading {}: {e}", current.display()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("reading {}: {e}", current.display()))?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name == "target" || name == ".git" {
                continue;
            }
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let rel = path.strip_prefix(dir).unwrap_or(&path);
            let bytes = std::fs::read(&path).map_err(|e| format!("reading {}: {e}", path.display()))?;
            acc.push_str(&format!(
                "{}|{}\n",
                rel.display(),
                stable_hash(&String::from_utf8_lossy(&bytes))
            ));
        }
    }
    Ok(stable_hash(&acc))
}

/// The dep's crate directory within its materialized source.
fn crate_dir_of(dep: &NativeDep, source_root: &Path) -> PathBuf {
    match dep.crate_subdir.as_deref() {
        Some(sub) => source_root.join(sub),
        None => source_root.to_path_buf(),
    }
}

/// Put the dep's source on disk and return its root.
///
/// For a pinned dep this fetches (network only when the commit is missing) and
/// materializes a files-only checkout; a working tree is already on disk.
fn materialize_source(
    source: &NativeSource,
    commit_override: Option<&str>,
) -> Result<PathBuf, String> {
    match source {
        NativeSource::Pinned { url, sha } => {
            let commit = commit_override.unwrap_or(sha.as_ref());
            cljrs_project::vcs::fetch_remote(url, commit).map_err(|e| e.to_string())?;
            cljrs_project::vcs::worktree_at_commit(url, commit).map_err(|e| e.to_string())
        }
        NativeSource::WorkingTree(root) => {
            if commit_override.is_some() {
                return Err("a :local/root dep has no commit to build at".into());
            }
            if !root.is_dir() {
                return Err(format!("local dep root not found at {}", root.display()));
            }
            Ok(root.canonicalize().unwrap_or_else(|_| root.clone()))
        }
    }
}

/// Derive every path the build needs. Pure: the source is already resolved.
fn plan_wrapper(dep: &NativeDep, crate_dir: PathBuf, version: &SourceVersion) -> WrapperPlan {
    let label = format!("{}{}", dep.pkg_ident(), version.slug);
    let fp_hash = stable_hash(&format!("{}|{}", abi_fingerprint(), version.key));
    let wrapper_dir = dylib_cache_root()
        .join(&label)
        .join(format!("fp-{fp_hash}"));
    let artifact = wrapper_artifact_path(&wrapper_dir);
    WrapperPlan {
        crate_dir,
        wrapper_dir,
        artifact,
        label,
    }
}

/// Run `cargo build` on the generated wrapper, matching the host's profile
/// (see [`abi_fingerprint`]).
fn cargo_build(plan: &WrapperPlan) -> Result<(), String> {
    let mut cmd = std::process::Command::new("cargo");
    cmd.arg("build").current_dir(&plan.wrapper_dir);
    if host_profile() == "release" {
        cmd.arg("--release");
    }
    if find_workspace_root().is_some() {
        cmd.arg("--offline");
    }
    eprintln!("[cljrs] building native package {}…", plan.label);
    let status = cmd.status().map_err(|e| format!("cargo: {e}"))?;
    if !status.success() {
        return Err(format!(
            "cargo build of native wrapper failed (see output above; wrapper at {})",
            plan.wrapper_dir.display()
        ));
    }
    Ok(())
}

/// Write the generated wrapper crate (Cargo.toml, build.rs, src/lib.rs).
///
/// The dependency is declared under the Rust identifier the `:rust/init` path
/// uses, renamed to the package's real name — those differ whenever a package
/// name contains `-`, which Rust identifiers cannot.
fn write_wrapper_crate(
    wrapper_dir: &Path,
    crate_dir: &Path,
    dep: &NativeDep,
) -> Result<(), String> {
    let pkg_ident = dep.pkg_ident();
    let manifest = std::fs::read_to_string(crate_dir.join("Cargo.toml"))
        .map_err(|e| format!("reading {}: {e}", crate_dir.join("Cargo.toml").display()))?;
    let package_name = package_name_of(&manifest).ok_or_else(|| {
        format!(
            "no [package] name in {}",
            crate_dir.join("Cargo.toml").display()
        )
    })?;
    let dep_line = format!(
        r#"{pkg_ident} = {{ path = "{}", package = "{package_name}" }}"#,
        crate_dir.display()
    );
    std::fs::create_dir_all(wrapper_dir.join("src")).map_err(|e| e.to_string())?;

    // Pin cljrs-interop exactly like the AOT harness pins runtime crates:
    // a local checkout when one is found (offline), the published version
    // otherwise.  The handshake catches any residual mismatch.
    let interop_dep = match find_workspace_root() {
        Some(root) => format!(
            "cljrs-interop = {{ path = \"{}\" }}",
            root.join("crates/cljrs-interop").display()
        ),
        None => format!("cljrs-interop = \"={}\"", env!("CARGO_PKG_VERSION")),
    };

    let cargo_toml = format!(
        r#"[package]
name = "cljrs-pinned-wrapper"
version = "{version}"
edition = "2024"

[workspace]

[lib]
crate-type = ["cdylib"]

[dependencies]
{interop_dep}
{dep_line}

[profile.release]
panic = "unwind"
"#,
        version = env!("CARGO_PKG_VERSION"),
    );
    std::fs::write(wrapper_dir.join("Cargo.toml"), cargo_toml).map_err(|e| e.to_string())?;

    let build_rs = r#"fn main() {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let version = std::process::Command::new(rustc)
        .arg("-V")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    println!("cargo:rustc-env=CLJRS_WRAPPER_RUSTC={version}");
}
"#;
    std::fs::write(wrapper_dir.join("build.rs"), build_rs).map_err(|e| e.to_string())?;

    let lib_rs = format!(
        r#"//! Auto-generated pinned-package wrapper (cljrs).

/// ABI fingerprint baked at build time; must equal the host's
/// `cljrs::native::pinned::abi_fingerprint()` exactly (including the build profile —
/// cljrs-gc object headers differ between debug and release).
#[cfg(debug_assertions)]
static ABI: &str = concat!(
    "cljrs ",
    env!("CARGO_PKG_VERSION"),
    "; ",
    env!("CLJRS_WRAPPER_RUSTC"),
    "; debug\0"
);
#[cfg(not(debug_assertions))]
static ABI: &str = concat!(
    "cljrs ",
    env!("CARGO_PKG_VERSION"),
    "; ",
    env!("CLJRS_WRAPPER_RUSTC"),
    "; release\0"
);

#[unsafe(no_mangle)]
pub extern "C" fn cljrs_dylib_abi() -> *const std::os::raw::c_char {{
    ABI.as_ptr() as *const std::os::raw::c_char
}}

/// Register the pinned package's exports into the host-provided registry.
///
/// # Safety
/// `registry` must be a valid `*mut cljrs_interop::Registry` from a host
/// whose ABI fingerprint matched `cljrs_dylib_abi()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cljrs_dylib_init(registry: *mut cljrs_interop::Registry) {{
    let registry = unsafe {{ &mut *registry }};
    // This dylib's own #[export] inventory (separate from the host's).
    cljrs_interop::register_exports(registry);
    {pkg_ident}::{init_tail}(registry);
}}
"#,
        init_tail = dep.init_tail(),
    );
    std::fs::write(wrapper_dir.join("src/lib.rs"), lib_rs).map_err(|e| e.to_string())?;
    Ok(())
}

/// The `[package] name` declared in a Cargo manifest.
///
/// Only that one key is read, so the full TOML grammar is not needed: track
/// which table each line belongs to and take `name` from `[package]`.
fn package_name_of(manifest: &str) -> Option<String> {
    let mut in_package = false;
    for line in manifest.lines() {
        let line = line.split('#').next().unwrap_or(line).trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "name" {
            continue;
        }
        let value = value.trim().trim_matches(['"', '\''].as_slice());
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

/// The built artifact path for a wrapper crate dir (host-profile build).
fn wrapper_artifact_path(wrapper_dir: &Path) -> PathBuf {
    let stem = "cljrs_pinned_wrapper";
    #[cfg(target_os = "macos")]
    let file = format!("lib{stem}.dylib");
    #[cfg(target_os = "windows")]
    let file = format!("{stem}.dll");
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let file = format!("lib{stem}.so");
    wrapper_dir.join("target").join(host_profile()).join(file)
}

// ── Loading ───────────────────────────────────────────────────────────────────

/// dlopen the wrapper, verify the ABI handshake, and run its init.
///
/// `version` selects the `Registry` view: `Some(commit)` registers the
/// package's exports into the immutable `"<ns>@<commit>"` namespace (pinned
/// versioned resolution); `None` registers into the live, unversioned
/// namespaces (the plain-`require` path).
fn load_library(
    globals: &Arc<GlobalEnv>,
    path: &Path,
    version: Option<&str>,
) -> Result<(), String> {
    // SAFETY: the library is generated by `write_wrapper_crate` and built by
    // us; the Rust-ABI init call is guarded by the fingerprint handshake.
    unsafe {
        let lib = libloading::Library::new(path).map_err(|e| e.to_string())?;

        let abi: libloading::Symbol<unsafe extern "C" fn() -> *const std::os::raw::c_char> =
            lib.get(ABI_SYMBOL).map_err(|e| e.to_string())?;
        let got = std::ffi::CStr::from_ptr(abi())
            .to_string_lossy()
            .to_string();
        let expected = abi_fingerprint();
        if got != expected {
            return Err(format!(
                "ABI fingerprint mismatch: wrapper was built as `{got}` but this binary \
                 expects `{expected}`; rebuild with the matching toolchain/cljrs version"
            ));
        }

        let init: libloading::Symbol<unsafe extern "C" fn(*mut cljrs_interop::Registry)> =
            lib.get(INIT_SYMBOL).map_err(|e| e.to_string())?;
        let mut registry = match version {
            Some(commit) => cljrs_interop::Registry::versioned(globals.clone(), commit),
            None => cljrs_interop::Registry::for_require(globals.clone()),
        };
        init(&mut registry as *mut _);

        // The dylib's code must stay mapped as long as any registered
        // NativeFn closure exists (same contract as `super::load_project_lib`).
        std::mem::forget(lib);
    }
    Ok(())
}

// ── Paths & misc ──────────────────────────────────────────────────────────────

/// `~/.cljrs/cache/dylibs`.
fn dylib_cache_root() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".cljrs").join("cache").join("dylibs")
}

/// Locate a local clojurust checkout for path-pinned wrapper deps:
/// `CLJRS_WORKSPACE_ROOT` override first, then this crate's compile-time
/// manifest location (`<workspace>/crates/cljrs`).
fn find_workspace_root() -> Option<PathBuf> {
    let validate = |p: PathBuf| -> Option<PathBuf> {
        (p.join("Cargo.toml").exists() && p.join("crates/cljrs-interop/Cargo.toml").exists())
            .then_some(p)
    };
    if let Some(root) = std::env::var_os("CLJRS_WORKSPACE_ROOT") {
        return validate(PathBuf::from(root));
    }
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    validate(manifest_dir.parent()?.parent()?.to_path_buf())
}

/// Short stable hex hash for cache directory names.
fn stable_hash(s: &str) -> String {
    use std::hash::{DefaultHasher, Hash as _, Hasher as _};
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    format!("{:016x}", h.finish())
}

#[cfg(test)]
mod tests {
    use super::package_name_of;

    #[test]
    fn reads_the_package_name() {
        let manifest = r#"
[package]
name = "cljrs-raster"
version = "0.1.0"
"#;
        assert_eq!(package_name_of(manifest).as_deref(), Some("cljrs-raster"));
    }

    #[test]
    fn ignores_a_name_in_another_table() {
        // `[lib] name` and a dependency's `package` key are both `name`-ish
        // and both wrong; only `[package]` decides.
        let manifest = r#"
[lib]
name = "wrong_lib_name"

[package]
name = "right-one"

[dependencies.serde]
name = "also-wrong"
"#;
        assert_eq!(package_name_of(manifest).as_deref(), Some("right-one"));
    }

    #[test]
    fn tolerates_comments_and_spacing() {
        let manifest = r#"
# leading comment
[package]   # the package table
   name   =   'spaced-out'   # trailing comment
"#;
        assert_eq!(package_name_of(manifest).as_deref(), Some("spaced-out"));
    }

    #[test]
    fn a_manifest_without_a_package_name_yields_none() {
        assert_eq!(package_name_of("[workspace]\nmembers = []\n"), None);
        assert_eq!(package_name_of(""), None);
    }

    #[test]
    fn a_workspace_inherited_version_does_not_confuse_it() {
        let manifest = r#"
[package]
name = "cljrs-ffmpeg"
version.workspace = true
edition.workspace = true
"#;
        assert_eq!(package_name_of(manifest).as_deref(), Some("cljrs-ffmpeg"));
    }
}
