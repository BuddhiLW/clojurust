//! The suite that deliberately pays for a runtime, so a startup regression
//! stays visible.
//!
//! Every other property suite in this crate now shares one runtime per test
//! thread and isolates cases by namespace, which took `defonce_metadata_properties`
//! from 36s to 0.7s and `meta_golden` from 49s to 1.3s. That reuse has a cost
//! of its own: per-case construction is *how the last startup regression was
//! noticed*. It multiplied a 0.2s bootstrap by a few hundred cases and turned
//! a barely-perceptible slowdown into a two-minute suite that nobody could
//! ignore. Reuse everywhere would have hidden it.
//!
//! So one measurement is kept, stated directly rather than as a side effect of
//! a slow suite: building a runtime is timed, and a regression fails here with
//! the number in the message instead of showing up as CI getting slower.
//!
//! The threshold is 150ms in a debug build. Measured cost on this tree is
//! ~40ms; the regression this replaces was 130-250ms, so the bound sits above
//! the true cost with room for a loaded machine and below the band that was
//! actually a problem. It is deliberately not a benchmark: it answers "did
//! bootstrap cost jump by an order of magnitude", which is the question that
//! has twice been worth asking.

use std::time::{Duration, Instant};

/// How long one runtime takes to build, including the eager `clojure.test`
/// evaluation that the property suites ask for (its defmethods are a large
/// part of what regressed last time).
fn build_once() -> Duration {
    let start = Instant::now();
    let runtime = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
        .eager_clojure_test(true)
        .build()
        .expect("runtime");
    let elapsed = start.elapsed();
    // Keep it alive until after the measurement, so a future change that moves
    // work into Drop cannot be timed away.
    drop(runtime);
    elapsed
}

#[test]
fn building_a_runtime_stays_cheap_enough_to_do_per_case() {
    // The best of several, not the mean: this runs on a shared machine, where
    // a scheduler hiccup inflates a sample but nothing deflates one. The
    // minimum is the closest thing to the true cost, and a real regression
    // raises the minimum along with everything else.
    let best = (0..5).map(|_| build_once()).min().expect("five samples");

    assert!(
        best < Duration::from_millis(150),
        "building a runtime took {best:?}, over the 150ms bound. \
         It cost ~40ms when this bound was set. Startup is multiplied by every \
         suite that builds per case, so treat this as a real regression and \
         find what got evaluated at build time, rather than raising the bound."
    );
}

/// A fresh runtime really is fresh: whatever the shared-runtime suites do to
/// their namespaces, this one starts from the bootstrap and nothing else.
#[test]
fn a_fresh_runtime_has_only_what_the_bootstrap_put_there() {
    use cljrs_runtime::env::env::Env;

    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
        .build()
        .expect("runtime")
        .into_globals();
    let mut env = Env::new(globals.clone(), "user");

    let mut parser = cljrs_reader::Parser::new(
        "[(bound? (resolve 'inc)) (nil? (resolve 'pt-nothing-defines-this))]".to_string(),
        "<canary>".to_string(),
    );
    let forms = parser.parse_all().expect("parse");
    let mut result = cljrs_value::Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(&form, &mut env).expect("eval");
    }

    assert_eq!(format!("{result}"), "[true true]");
}
