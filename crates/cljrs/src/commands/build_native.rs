//! `cljrs build-native` — build the project's own `:rust` crate as a shared
//! library, the one `cljrs run` and `cljrs repl` load at startup.

use miette::IntoDiagnostic as _;

use crate::native;

#[derive(clap::Args)]
pub struct Args {
    /// Build in release mode instead of debug.
    #[arg(long)]
    pub release: bool,
}

/// Build the native Rust crate declared in `cljrs.edn` as a shared library.
pub fn run(args: Args) -> miette::Result<i32> {
    let release = args.release;
    let cwd = std::env::current_dir().into_diagnostic()?;
    let config = cljrs_project::config::load_config(&cwd)
        .into_diagnostic()?
        .ok_or_else(|| miette::miette!("no cljrs.edn found in or above the current directory"))?;

    let rust_config = config
        .rust
        .as_ref()
        .ok_or_else(|| miette::miette!("no :rust key found in cljrs.edn"))?;

    let crate_name = rust_config
        .crate_name()
        .ok_or_else(|| miette::miette!(":rust has no :init function; cannot derive crate name"))?;

    eprintln!(
        "[build-native] building {} in {}",
        crate_name,
        rust_config.crate_dir.display()
    );

    let mut cmd = std::process::Command::new("cargo");
    cmd.arg("build");
    if release {
        cmd.arg("--release");
    }
    cmd.current_dir(&rust_config.crate_dir);

    let status = cmd.status().into_diagnostic()?;
    if !status.success() {
        return Err(miette::miette!("cargo build failed"));
    }

    let lib_path = native::native_lib_path(&rust_config.crate_dir, crate_name, release);
    eprintln!("[build-native] built {}", lib_path.display());
    // Read the artifact, not this CLI's fingerprint: --release may build a
    // different profile from the running CLI, and old crates lack the symbol.
    unsafe {
        let lib = libloading::Library::new(&lib_path).into_diagnostic()?;
        match lib.get::<unsafe extern "C" fn() -> *const std::os::raw::c_char>(native::pinned::ABI_SYMBOL) {
            Ok(abi) => {
                let ptr = abi();
                if ptr.is_null() {
                    return Err(miette::miette!("native library returned a null ABI fingerprint"));
                }
                eprintln!("[build-native] ABI fingerprint: {}", std::ffi::CStr::from_ptr(ptr).to_string_lossy());
            }
            Err(_) => eprintln!("[build-native] ABI fingerprint: absent (legacy crate; use cljrs_interop::export_init!)"),
        }
    }
    println!("{}", lib_path.display());

    Ok(0)
}
