//! The `cljrs` command-line tool.
//!
//! The binary (`src/main.rs`) is a one-line shim over [`cli::main`]; everything
//! else lives here so the CLI's own integration tests can reach it. This is a
//! tool, not an embedding API — nothing here is a stable interface, and a host
//! that wants to run Clojure should build a `cljrs_runtime::Runtime` directly.
//!
//! | Module | Responsibility |
//! |---|---|
//! | [`cli`] | Argument definitions, process setup, and subcommand dispatch |
//! | [`commands`] | One module per subcommand: its arguments and its implementation |
//! | [`session`] | Building the runtime the commands evaluate in, and evaluating source in it |
//! | [`native`] | Loading native (Rust) code into a running environment |
//! | [`extensions`] | Which optional extensions this build supplies to the AOT compiler |

pub mod cli;
pub mod commands;
pub mod extensions;
pub mod native;
pub mod session;
