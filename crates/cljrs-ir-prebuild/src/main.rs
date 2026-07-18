//! Standalone `cljrs-ir-prebuild` CLI. Thin Clap wrapper over the
//! [`cljrs_ir_prebuild`] library; the same logic is exposed as the `cljrs
//! ir-prebuild` subcommand of the main `cljrs` binary.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// Pre-lower Clojure namespaces to serialized IR bundles.
#[derive(Parser)]
#[command(name = "cljrs-ir-prebuild")]
struct Cli {
    /// Namespaces to lower (e.g. "clojure.core"). If none given, defaults to clojure.core.
    #[arg(short, long)]
    ns: Vec<String>,

    /// Output file path for the serialized IR bundle.
    #[arg(short, long, default_value = "ir_bundle.bin")]
    output: PathBuf,

    /// Additional source paths for namespace resolution.
    #[arg(long)]
    src_path: Vec<PathBuf>,

    /// Print verbose progress information.
    #[arg(short, long)]
    verbose: bool,

    /// Subcommand, default is 'prebuild'
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    #[command(name = "prebuild")]
    Prebuild,

    Dump {
        input: PathBuf,
    },
}

fn main() {
    let cli = Cli::parse();

    let command = cli.command.unwrap_or(Commands::Prebuild);

    match command {
        Commands::Prebuild => {
            let namespaces = if cli.ns.is_empty() {
                vec!["clojure.core".to_string()]
            } else {
                cli.ns
            };
            // The compiler uses deep recursion; run on a large-stack thread.
            let result = std::thread::Builder::new()
                .name("prebuild-main".to_string())
                .stack_size(32 * 1024 * 1024)
                .spawn(move || {
                    cljrs_ir_prebuild::run_prebuild(
                        &namespaces,
                        &cli.output,
                        &cli.src_path,
                        cli.verbose,
                    )
                })
                .expect("failed to spawn prebuild thread")
                .join()
                .expect("prebuild thread panicked");

            match result {
                Ok(stats) => {
                    eprintln!(
                        "Wrote {} functions ({} unsupported) to {}",
                        stats.lowered,
                        stats.unsupported,
                        stats.output.display()
                    );
                }
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }

        Commands::Dump { input } => {
            let bytes = std::fs::read(input).expect("failed to read input file");
            let bundle =
                cljrs_ir::deserialize_bundle(&bytes).expect("failed to deserialize input file");
            println!("{}", bundle);
        }
    }
}
