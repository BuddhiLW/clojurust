//! Pre-lower Clojure namespaces to IR and serialize the result to a bundle.
//!
//! Boots a full eval environment, loads the Clojure compiler, then iterates
//! all vars in the requested namespaces. For each function, every arity is
//! lowered to IR and stored in an [`IrBundle`]. The bundle can be serialized
//! to a file that is later loaded at startup (`cljrs_eval::load_prebuilt_ir`)
//! to skip re-lowering already-compiled functions — most useful for cutting
//! cold-start latency on targets that can't run the background lowering
//! worker, such as an embedder built for `wasm32`.
//!
//! This crate is both a library (used by the `cljrs ir-prebuild` subcommand)
//! and a standalone `cljrs-ir-prebuild` binary.

use std::path::PathBuf;
use std::sync::Arc;

use cljrs_env::env::{Env, GlobalEnv};
use cljrs_ir::IrBundle;
use cljrs_value::{CljxFn, Value};

/// Outcome of a [`run_prebuild`] call.
pub struct PrebuildStats {
    /// Number of function arities successfully lowered to IR.
    pub lowered: usize,
    /// Number of function arities the lowerer could not handle.
    pub unsupported: usize,
    /// Where the serialized bundle was written.
    pub output: PathBuf,
}

/// Boot a standard environment, lower every function in `namespaces` to IR,
/// and write the serialized bundle to `output`.
///
/// Non-`clojure.core` namespaces are `require`d from `src_paths` before
/// lowering. Returns an error string on any unrecoverable failure (IR
/// lowering disabled, a namespace that fails to load, or an I/O error).
pub fn run_prebuild(
    namespaces: &[String],
    output: &PathBuf,
    src_paths: &[PathBuf],
    verbose: bool,
) -> Result<PrebuildStats, String> {
    // 1. Boot the environment.
    let globals = if src_paths.is_empty() {
        cljrs_eval::standard_env()
    } else {
        cljrs_eval::standard_env_with_paths(src_paths.to_vec())
    };

    let mut env = Env::new(globals.clone(), "user");

    // 2. Enable IR lowering.
    if !cljrs_eval::mark_compiler_ready(&globals) {
        return Err("IR lowering is disabled (CLJRS_NO_IR is set)".to_string());
    }

    // 3. Load any non-core namespaces that were requested.
    for ns_name in namespaces {
        if ns_name != "clojure.core" {
            load_namespace(&globals, &mut env, ns_name, verbose)?;
        }
    }

    // 4. Walk all vars and lower functions to IR.
    let mut bundle = IrBundle::new();
    let mut lowered = 0usize;
    let mut unsupported = 0usize;

    for ns_name in namespaces {
        if verbose {
            eprintln!("Lowering namespace: {ns_name}");
        }
        let (ns_lowered, ns_unsupported) =
            lower_namespace(&globals, &mut env, ns_name, &mut bundle, verbose)?;
        lowered += ns_lowered;
        unsupported += ns_unsupported;
    }

    if verbose {
        eprintln!("Lowering complete: {lowered} functions lowered, {unsupported} unsupported.");
    }

    // 5. Serialize and write to output file.
    let bytes =
        cljrs_ir::serialize_bundle(&bundle).map_err(|e| format!("serialization failed: {e}"))?;
    std::fs::write(output, &bytes)
        .map_err(|e| format!("failed to write {}: {e}", output.display()))?;

    if verbose {
        eprintln!("Wrote {} bytes to {}", bytes.len(), output.display());
    }

    Ok(PrebuildStats {
        lowered,
        unsupported,
        output: output.clone(),
    })
}

/// Load a namespace by evaluating `(require 'ns-name)`.
fn load_namespace(
    globals: &Arc<GlobalEnv>,
    env: &mut Env,
    ns_name: &str,
    verbose: bool,
) -> Result<(), String> {
    if verbose {
        eprintln!("Loading namespace: {ns_name}");
    }

    let span = cljrs_types::span::Span::new(Arc::new("<prebuild>".to_string()), 0, 0, 1, 1);
    let require_form = cljrs_reader::Form::new(
        cljrs_reader::form::FormKind::List(vec![
            cljrs_reader::Form::new(
                cljrs_reader::form::FormKind::Symbol("require".into()),
                span.clone(),
            ),
            cljrs_reader::Form::new(
                cljrs_reader::form::FormKind::Quote(Box::new(cljrs_reader::Form::new(
                    cljrs_reader::form::FormKind::Symbol(ns_name.into()),
                    span.clone(),
                ))),
                span,
            ),
        ]),
        cljrs_types::span::Span::new(Arc::new("<prebuild>".to_string()), 0, 0, 1, 1),
    );

    cljrs_eval::eval(&require_form, env)
        .map_err(|e| format!("failed to load namespace {ns_name}: {e:?}"))?;

    if !globals.is_loaded(ns_name) {
        return Err(format!(
            "namespace {ns_name} was not marked as loaded after require"
        ));
    }

    Ok(())
}

/// Lower all functions in a namespace to IR and store them in the bundle.
/// Returns (lowered_count, unsupported_count).
fn lower_namespace(
    globals: &Arc<GlobalEnv>,
    env: &mut Env,
    ns_name: &str,
    bundle: &mut IrBundle,
    verbose: bool,
) -> Result<(usize, usize), String> {
    // Collect all var names and their values from the namespace's interns.
    let var_entries: Vec<(Arc<str>, Value)> = {
        let ns_map = globals.namespaces.read().unwrap();
        let ns = ns_map
            .get(ns_name)
            .ok_or_else(|| format!("namespace {ns_name} not found"))?;
        let interns = ns.get().interns.lock().unwrap();
        interns
            .iter()
            .map(|(name, var)| {
                let val = var.get().deref().unwrap_or(Value::Nil);
                (name.clone(), val)
            })
            .collect()
    };

    let mut lowered = 0usize;
    let mut unsupported = 0usize;

    for (var_name, val) in &var_entries {
        let f = match val {
            Value::Fn(gc_fn) => gc_fn.get().clone(),
            _ => continue,
        };

        // Skip macros — they operate on forms, not values.
        if f.is_macro {
            continue;
        }

        let fn_lowered = lower_function(ns_name, var_name, &f, env, bundle, verbose);
        lowered += fn_lowered.0;
        unsupported += fn_lowered.1;
    }

    if verbose {
        eprintln!("  {ns_name}: {lowered} lowered, {unsupported} unsupported");
    }

    Ok((lowered, unsupported))
}

/// Lower all arities of a single function.
/// Returns (lowered_count, unsupported_count).
fn lower_function(
    ns_name: &str,
    var_name: &str,
    f: &CljxFn,
    env: &mut Env,
    bundle: &mut IrBundle,
    verbose: bool,
) -> (usize, usize) {
    let mut lowered = 0;
    let mut unsupported = 0;

    for arity in &f.arities {
        let param_count = arity.params.len();
        let is_variadic = arity.rest_param.is_some();

        // Build a stable key: "ns/name:param_count" or "ns/name:param_count+"
        // for variadic arities. If there are multiple arities with different
        // param counts, each gets a unique key.
        let key = if is_variadic {
            format!("{ns_name}/{var_name}:{param_count}+")
        } else {
            format!("{ns_name}/{var_name}:{param_count}")
        };

        let ns_arc: Arc<str> = Arc::from(ns_name);
        match cljrs_eval::lower::lower_arity(
            f.name.as_deref(),
            &arity.params,
            arity.rest_param.as_ref(),
            &arity.destructure_params,
            arity.destructure_rest.as_ref(),
            &arity.body,
            &ns_arc,
            env,
            f.is_async,
        ) {
            Ok(ir_func) => {
                if verbose {
                    eprintln!("    lowered {key} ({} blocks)", ir_func.blocks.len());
                }
                bundle.insert(key, ir_func);
                lowered += 1;
            }
            Err(e) => {
                if verbose {
                    eprintln!("    unsupported {key}: {e}");
                }
                unsupported += 1;
            }
        }
    }

    (lowered, unsupported)
}
