//! The `cljrs` binary. Everything it does lives in the library beside it, so
//! the CLI's integration tests can reach the same code.

fn main() -> miette::Result<()> {
    cljrs::cli::main()
}
