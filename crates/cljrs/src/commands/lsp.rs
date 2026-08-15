//! `cljrs lsp` — run the language server over stdio.

pub fn run() -> miette::Result<i32> {
    // cljrs-lsp owns its own runtime; this is safe because the CLI never
    // enters the interpreter's async driver runtime on this path.
    cljrs_lsp::run_stdio_blocking().map_err(|e| miette::miette!("lsp server error: {e}"))?;
    Ok(0)
}
