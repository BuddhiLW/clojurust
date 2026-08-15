//! Diagnostic logging configuration: `tracing` targets and filters.
//!
//! The runtime, the GC, and the compiler emit their internal diagnostics with
//! plain `tracing::debug!` / `tracing::trace!` under a small set of **feature
//! targets** — see [`FEATURE_TARGETS`]. Selecting them is a filter, not an API:
//! anything that can build a [`Targets`] filter can turn them on.
//!
//! Two entry points build that filter for the two hosts that ship in this
//! workspace:
//!
//! * The `cljrs` CLI starts from [`base_filter`] (its `--debug`/`--trace`
//!   level, with the feature targets pinned off and the codegen crates pinned
//!   to `warn`), layers each `-X debug:gc,jit` flag on with [`apply_x_flag`],
//!   and installs the result with [`init`].
//! * A generated AOT harness calls [`init_from_env`], which enables *nothing*
//!   unless `CLJRS_X_FLAG` or `RUST_LOG` asks for it.
//!
//! An embedding host is free to ignore all of this and install its own
//! subscriber; the emitting code has no opinion.
//!
//! This module is native-only: `tracing-subscriber` is a host-side concern and
//! a `wasm32` runtime installs no subscriber. The `tracing::debug!` call sites
//! themselves compile everywhere and are inert without one.

use tracing::level_filters::LevelFilter;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

/// Targets carrying the runtime's own internal diagnostics.
///
/// These are firehoses — `gc` logs every collection decision, `env` every
/// symbol lookup — so [`base_filter`] pins them off rather than letting a
/// blanket `--debug` turn them all on at once. Name the ones you want:
/// `-X debug:gc,jit`, `CLJRS_X_FLAG=trace:env`, or `RUST_LOG=gc=debug`.
///
/// | Target | Emitted by |
/// |---|---|
/// | `gc` | `cljrs-gc`: collection cycles, region allocation |
/// | `env` | `cljrs-runtime::env`: symbol lookup |
/// | `ir` | `cljrs-runtime::tiered`: lowering, IR interpretation, cache eviction |
/// | `jit` | `cljrs-runtime::tiered` and `cljrs-compiler::jit`: promotion, compilation, code-cache reclamation |
pub const FEATURE_TARGETS: &[&str] = &["gc", "env", "ir", "jit"];

/// Crates whose logging is noisy enough to drown out everything else.
///
/// Cranelift (and its register allocator) log whole function bodies of IR at
/// `info`/`debug` through the `log` crate, which `tracing-subscriber`'s
/// `tracing-log` bridge forwards into our subscriber. A single JIT compile
/// therefore buries any real message. [`base_filter`] pins these to `warn`
/// regardless of the requested default level; set `RUST_LOG` to see them.
pub const NOISY_TARGETS: &[&str] = &[
    "cranelift_codegen",
    "cranelift_frontend",
    "cranelift_jit",
    "cranelift_module",
    "cranelift_native",
    "cranelift_object",
    "regalloc2",
];

/// The filter a host starts from: everything at `default`, the codegen crates
/// pinned to `warn`, and the runtime's [`FEATURE_TARGETS`] pinned off.
pub fn base_filter(default: impl Into<LevelFilter>) -> Targets {
    let mut filter = Targets::new().with_default(default.into());
    for target in NOISY_TARGETS {
        filter = filter.with_target(*target, LevelFilter::WARN);
    }
    for target in FEATURE_TARGETS {
        filter = filter.with_target(*target, LevelFilter::OFF);
    }
    filter
}

/// Fold one `-X` / `CLJRS_X_FLAG` spec into `filter`.
///
/// Format: `<level>:<target1>,<target2>,…` where `<level>` is `debug` or
/// `trace`, e.g. `debug:gc,jit` or `trace:env`. Any target name is accepted,
/// including one nothing ever logs to.
///
/// Returns `Err` with a message if the format is invalid.
pub fn apply_x_flag(mut filter: Targets, spec: &str) -> Result<Targets, String> {
    let (level_str, targets) = spec
        .split_once(':')
        .ok_or_else(|| format!("expected <level>:<targets>, got: {spec}"))?;

    let level = match level_str {
        "debug" => LevelFilter::DEBUG,
        "trace" => LevelFilter::TRACE,
        other => {
            return Err(format!(
                "unknown level '{other}', expected 'debug' or 'trace'"
            ));
        }
    };

    for target in targets.split(',') {
        let target = target.trim();
        if target.is_empty() {
            continue;
        }
        filter = filter.with_target(target.to_string(), level);
    }
    Ok(filter)
}

/// Install `filter` as the process's global subscriber, formatting to stderr.
///
/// Idempotent in the sense that a second call (or a host that installed its
/// own subscriber first) is ignored rather than panicking.
pub fn init(filter: Targets) {
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .try_init();
}

/// Replace `base` with `RUST_LOG`'s filter when that variable is set and
/// non-empty.
///
/// A spec that does not parse is reported on stderr and `base` is kept.
/// `RUST_LOG` belongs to the ecosystem, not to us — plenty of crates read it,
/// and a value we reject may be aimed at one of them — so an unusable value
/// degrades the host to its own default rather than killing the process or
/// silencing it. Both hosts in this workspace use this, so
/// `RUST_LOG=<garbage>` means the same thing to the CLI and to an AOT binary.
pub fn apply_rust_log(base: Targets) -> Targets {
    let Ok(spec) = std::env::var("RUST_LOG") else {
        return base;
    };
    if spec.trim().is_empty() {
        return base;
    }
    match spec.parse::<Targets>() {
        Ok(filter) => filter,
        Err(e) => {
            // No subscriber is installed yet, so this cannot go through `tracing`.
            eprintln!("cljrs: ignoring RUST_LOG ({e}); using the default log filter");
            base
        }
    }
}

/// Install a subscriber configured entirely from the environment.
///
/// Nothing is enabled by default — a binary that sets neither variable logs
/// exactly as much as one with no subscriber at all. `RUST_LOG` is a full
/// [`Targets`] spec (`gc=debug,cranelift_codegen=info`); `CLJRS_X_FLAG` names
/// feature targets ([`apply_x_flag`]) and is layered on top, so both can be
/// used together.
///
/// This is what a generated AOT harness calls, so `CLJRS_X_FLAG=debug:gc
/// ./my-binary` behaves the same as `cljrs -X debug:gc run my-app.cljrs` —
/// including on a value that does not parse. `CLJRS_X_FLAG` is ours alone and
/// has exactly one meaning, so a bad one is an `Err` here just as a bad `-X` is
/// a hard error in the CLI: the caller reports it and exits, because leaving
/// someone who asked for diagnostics with none is the one outcome nobody wants.
/// (`RUST_LOG` is treated more leniently — see [`apply_rust_log`].)
pub fn init_from_env() -> Result<(), String> {
    let mut filter = apply_rust_log(Targets::new());
    if let Ok(spec) = std::env::var("CLJRS_X_FLAG")
        && !spec.trim().is_empty()
    {
        filter = apply_x_flag(filter, &spec).map_err(|e| format!("invalid CLJRS_X_FLAG: {e}"))?;
    }
    init(filter);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::Level;

    /// A target the flag does not name keeps whatever the base filter gave it;
    /// the ones it names are raised to the requested level.
    #[test]
    fn x_flag_raises_only_the_named_targets() {
        let filter = apply_x_flag(base_filter(Level::INFO), "debug:gc,jit").unwrap();
        assert!(filter.would_enable("gc", &Level::DEBUG));
        assert!(filter.would_enable("jit", &Level::DEBUG));
        // Named at debug, so trace stays off.
        assert!(!filter.would_enable("gc", &Level::TRACE));
        // Not named: still pinned off by `base_filter`.
        assert!(!filter.would_enable("env", &Level::DEBUG));
    }

    #[test]
    fn trace_level_enables_debug_too() {
        let filter = apply_x_flag(Targets::new(), "trace:env").unwrap();
        assert!(filter.would_enable("env", &Level::TRACE));
        assert!(filter.would_enable("env", &Level::DEBUG));
    }

    /// A blanket `--debug` must not turn the runtime firehoses on; only `-X`
    /// does. Ordinary crate targets still follow the default level.
    #[test]
    fn base_filter_pins_feature_and_noisy_targets() {
        let filter = base_filter(Level::DEBUG);
        for target in FEATURE_TARGETS {
            assert!(
                !filter.would_enable(target, &Level::DEBUG),
                "{target} must stay off under a blanket --debug"
            );
        }
        assert!(!filter.would_enable("cranelift_codegen", &Level::INFO));
        assert!(filter.would_enable("cranelift_codegen", &Level::WARN));
        assert!(filter.would_enable("some_other_crate", &Level::DEBUG));
    }

    #[test]
    fn malformed_x_flags_are_rejected() {
        assert!(apply_x_flag(Targets::new(), "bogus").is_err());
        assert!(apply_x_flag(Targets::new(), "warn:gc").is_err());
    }

    /// `RUST_LOG` is process-global state, so these cases share one test rather
    /// than racing each other across the thread pool.
    #[test]
    fn rust_log_replaces_the_base_but_never_silences_it() {
        // SAFETY: single-threaded within this test; no other test reads RUST_LOG.
        let restore = std::env::var("RUST_LOG").ok();

        unsafe { std::env::remove_var("RUST_LOG") };
        assert!(
            apply_rust_log(base_filter(Level::INFO)).would_enable("anything", &Level::INFO),
            "unset RUST_LOG keeps the base filter"
        );

        unsafe { std::env::set_var("RUST_LOG", "   ") };
        assert!(
            apply_rust_log(base_filter(Level::INFO)).would_enable("anything", &Level::INFO),
            "blank RUST_LOG keeps the base filter"
        );

        unsafe { std::env::set_var("RUST_LOG", "gc=debug") };
        let parsed = apply_rust_log(base_filter(Level::INFO));
        assert!(
            parsed.would_enable("gc", &Level::DEBUG),
            "a valid RUST_LOG replaces the base, unpinning the feature targets"
        );

        // The point of the fallback: a typo degrades to the host's default
        // rather than to silence, and it degrades the same way for every host.
        unsafe { std::env::set_var("RUST_LOG", "=[not a filter]=") };
        let fallback = apply_rust_log(base_filter(Level::INFO));
        assert!(
            fallback.would_enable("anything", &Level::INFO),
            "an unparseable RUST_LOG must not silence the process"
        );
        assert!(
            !fallback.would_enable("gc", &Level::DEBUG),
            "...and must leave the base filter's pins intact"
        );

        match restore {
            Some(v) => unsafe { std::env::set_var("RUST_LOG", v) },
            None => unsafe { std::env::remove_var("RUST_LOG") },
        }
    }
}
