//! `cljrs eval` — evaluate one expression and print the result.

use std::sync::Arc;

use cljrs_gc::GcConfig;
use cljrs_value::Value;

use crate::session::{self, VersioningFlags};

#[derive(clap::Args)]
pub struct Args {
    /// The expression to evaluate.
    pub expr: String,
}

pub fn run(args: Args, versioning: VersioningFlags) -> miette::Result<i32> {
    let gc_config = Arc::new(GcConfig::new());
    let globals = session::setup_globals(Vec::new(), gc_config, versioning);
    let result = session::eval_source(&args.expr, "<eval>", globals)?;
    if result != Value::Nil {
        println!("{}", result);
    }
    Ok(0)
}
