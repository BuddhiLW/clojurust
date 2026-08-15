//! `cljrs deps` — inspect and fetch the dependencies declared in `cljrs.edn`.

use miette::IntoDiagnostic as _;

#[derive(clap::Subcommand)]
pub enum DepsCommands {
    /// Clone or update git dependencies from cljrs.edn.
    ///
    /// Without a name, fetches every git dependency declared in the
    /// nearest cljrs.edn.  With a name, fetches only that dependency.
    Fetch {
        /// Dependency name to fetch (fetches all if omitted).
        name: Option<String>,
    },
    /// Show which dependencies are cached and which are missing.
    Status,
}

/// Fetch one or all git dependencies declared in the nearest `cljrs.edn`.
fn run_deps_fetch(name: Option<String>) -> miette::Result<i32> {
    let cwd = std::env::current_dir().into_diagnostic()?;
    let config = cljrs_project::config::load_config(&cwd)
        .into_diagnostic()?
        .ok_or_else(|| miette::miette!("no cljrs.edn found in or above the current directory"))?;

    if config.deps.is_empty() {
        println!("No dependencies declared in cljrs.edn.");
        return Ok(0);
    }

    // Collect (dep_name, dependency) pairs to process.
    let to_fetch: Vec<(&str, &cljrs_project::config::Dependency)> = if let Some(ref n) = name {
        match config.find_dep(n) {
            Some(dep) => vec![(n.as_str(), dep)],
            None => {
                return Err(miette::miette!("dependency {:?} not found in cljrs.edn", n));
            }
        }
    } else {
        config.deps.iter().map(|(n, d)| (n.as_ref(), d)).collect()
    };

    let mut all_ok = true;
    for (dep_name, dep) in to_fetch {
        match dep {
            cljrs_project::config::Dependency::Git(git_dep) => {
                eprintln!("fetching {dep_name} ({})...", git_dep.url);
                match cljrs_project::vcs::fetch_remote(&git_dep.url, &git_dep.sha) {
                    Ok(path) => eprintln!("  ok → {}", path.display()),
                    Err(e) => {
                        eprintln!("  error: {e}");
                        all_ok = false;
                    }
                }
            }
            cljrs_project::config::Dependency::Local { root } => {
                if root.exists() {
                    eprintln!("{dep_name}: local dep at {} — ok", root.display());
                } else {
                    eprintln!(
                        "{dep_name}: local dep at {} — directory not found",
                        root.display()
                    );
                    all_ok = false;
                }
            }
        }
    }

    Ok(if all_ok { 0 } else { 1 })
}

/// Print the cache status of every dependency declared in the nearest `cljrs.edn`.
fn run_deps_status() -> miette::Result<i32> {
    let cwd = std::env::current_dir().into_diagnostic()?;
    let config = cljrs_project::config::load_config(&cwd)
        .into_diagnostic()?
        .ok_or_else(|| miette::miette!("no cljrs.edn found in or above the current directory"))?;

    if config.deps.is_empty() {
        println!("No dependencies declared in cljrs.edn.");
        return Ok(0);
    }

    let mut all_ok = true;
    for (dep_name, dep) in &config.deps {
        match dep {
            cljrs_project::config::Dependency::Git(git_dep) => {
                let cache_path = cljrs_project::vcs::cache_path_for_url(&git_dep.url);
                let sha_present = cache_path.exists()
                    && std::process::Command::new("git")
                        .arg("-C")
                        .arg(&cache_path)
                        .arg("cat-file")
                        .arg("-e")
                        .arg(git_dep.sha.as_ref())
                        .status()
                        .map(|s| s.success())
                        .unwrap_or(false);
                if sha_present {
                    println!(
                        "{dep_name}: cached (sha: {}, url: {})",
                        git_dep.sha, git_dep.url
                    );
                } else {
                    println!(
                        "{dep_name}: NOT cached — run `cljrs deps fetch` (sha: {}, url: {})",
                        git_dep.sha, git_dep.url
                    );
                    all_ok = false;
                }
            }
            cljrs_project::config::Dependency::Local { root } => {
                if root.exists() {
                    println!("{dep_name}: local dep at {} — ok", root.display());
                } else {
                    println!("{dep_name}: local dep at {} — NOT FOUND", root.display());
                    all_ok = false;
                }
            }
        }
    }

    Ok(if all_ok { 0 } else { 1 })
}

/// Hand a `deps` subcommand to its implementation.
pub fn run(command: DepsCommands) -> miette::Result<i32> {
    match command {
        DepsCommands::Fetch { name } => run_deps_fetch(name),
        DepsCommands::Status => run_deps_status(),
    }
}
