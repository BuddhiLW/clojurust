//! `cljrs nrepl` — serve an nREPL session for editor integration.

use std::path::PathBuf;

use crate::session::{self, VersioningFlags};

/// How long the idle server drives async tasks before checking for a
/// request again: the most a request waits, and the idle wake-up period.
const IDLE_SLICE: std::time::Duration = std::time::Duration::from_millis(20);

#[derive(clap::Args)]
pub struct Args {
    /// Port to listen on (0 = let the OS pick a free port).
    #[arg(long, default_value_t = 0)]
    pub port: u16,
    /// Address to bind.
    #[arg(long, default_value = "127.0.0.1")]
    pub bind: String,
    /// Source directories to search when resolving `require`.
    #[arg(long = "src-path", value_name = "DIR")]
    pub src_paths: Vec<PathBuf>,
    /// GC soft memory limit in MB (triggers collection when exceeded).
    #[arg(long)]
    pub gc_soft_limit_mb: Option<usize>,
    /// GC hard memory limit in MB (forces collection when exceeded).
    #[arg(long)]
    pub gc_hard_limit_mb: Option<usize>,
}

pub fn run(args: Args, versioning: VersioningFlags) -> miette::Result<i32> {
    let Args {
        port,
        bind,
        src_paths,
        gc_soft_limit_mb,
        gc_hard_limit_mb,
    } = args;
    let gc_config = session::build_gc_config(gc_soft_limit_mb, gc_hard_limit_mb);
    let globals = session::setup_globals(src_paths, gc_config, versioning);
    let addr: std::net::SocketAddr = format!("{bind}:{port}")
        .parse()
        .map_err(|e| miette::miette!("invalid bind address {bind}:{port}: {e}"))?;
    let config = cljrs_nrepl::Config {
        addr,
        port_file: Some(PathBuf::from(".nrepl-port")),
    };
    let server = cljrs_nrepl::start(config, globals)?;
    // Editors (CIDER, Calva) parse this exact line to auto-connect.
    println!(
        "nREPL server started on port {0} on host {bind} - nrepl://{bind}:{0}",
        server.port()
    );
    // Evaluate forms through the CLI's async driver so core.async /
    // ^:async code makes progress, exactly like `cljrs repl`. Between
    // requests keep driving it, so a loop started from the REPL keeps
    // running while the editor is quiet.
    server.serve_with_idle(session::eval_form, IDLE_SLICE, session::drive_idle)?;
    Ok(0)
}
