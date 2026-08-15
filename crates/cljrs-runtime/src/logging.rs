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

/// Install a subscriber configured entirely from the environment.
///
/// Nothing is enabled by default — a binary that sets neither variable logs
/// exactly as much as one with no subscriber at all. `CLJRS_X_FLAG` names
/// feature targets ([`apply_x_flag`]); `RUST_LOG` is a full [`Targets`] spec
/// (`gc=debug,cranelift_codegen=info`) applied underneath it, so both can be
/// used together.
///
/// This is what a generated AOT harness calls, so `CLJRS_X_FLAG=debug:gc
/// ./my-binary` behaves the same as `cljrs -X debug:gc run my-app.cljrs`.
pub fn init_from_env() {
    let mut filter = match std::env::var("RUST_LOG") {
        Ok(spec) if !spec.trim().is_empty() => spec.parse::<Targets>().unwrap_or_default(),
        _ => Targets::new(),
    };
    if let Ok(spec) = std::env::var("CLJRS_X_FLAG")
        && let Ok(updated) = apply_x_flag(filter.clone(), &spec)
    {
        filter = updated;
    }
    init(filter);
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
}
