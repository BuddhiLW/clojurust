# cljrs-stdlib

Built-in standard library namespaces for clojurust, distributed as embedded
source + native Rust helpers.

## Status

Phase 8-ext.  Provides `clojure.string`, `clojure.set`, and `clojure.test` as
lazily-loaded built-ins; no filesystem dependency at runtime.

## Purpose

Clojurust has no classpath or JAR mechanism.  This crate solves the distribution
problem by embedding `.cljrs` source files via `include_str!` and registering
them in `GlobalEnv::builtin_sources` so that `(require '[clojure.string :as str])`
works out of the box in any binary that calls `cljrs_stdlib::install()`.

This crate is an *extension*: it does not construct runtimes and does not choose
execution modes. Since Stage 3 of
[`docs/crate-consolidation-plan.md`](../../docs/crate-consolidation-plan.md),
runtime construction belongs to `cljrs_runtime::Runtime::builder()`, and the
caller — the CLI, an embedding host, the AOT compiler — picks the mode, source
paths, and GC limits.

## File layout

```
src/
  lib.rs                  Public API: install() / register(), and the
                          embedded-source include_str! table
  string.rs               Native Rust implementations for clojure.string
  set.rs                  Native Rust implementations for clojure.set
  io.rs                   Native Rust implementations for clojure.rust.io
                          (IoReader/IoWriter/StringReader); not built for wasm32
  edn.rs                  Native Rust implementations for clojure.edn; not built for wasm32
  clojure/
    string.cljrs          Clojure source for clojure.string (ns decl; natives pre-registered)
    set.cljrs             Clojure source for clojure.set   (ns decl; natives pre-registered)
    template.cljrs        Pure Clojure implementation of clojure.template
    test.cljrs            Pure Clojure implementation of clojure.test
    walk.cljrs            Pure Clojure implementation of clojure.walk
    data.cljrs            Pure Clojure implementation of clojure.data
    zip.cljrs             Pure Clojure implementation of clojure.zip
    edn.cljrs             Clojure source for clojure.edn (ns decl; natives pre-registered)
    rust/
      io.cljrs            Clojure source for clojure.rust.io (ns decl; natives pre-registered)
    spec/
      alpha.cljrs         Pure Clojure implementation of clojure.spec.alpha (core spec engine)
      test/
        alpha.cljrs       Pure Clojure implementation of clojure.spec.test.alpha (instrument/unstrument)
      gen/
        alpha.cljrs       clojure.spec.gen.alpha stub — every fn throws (no generator engine)
```

There is no build script. Lowering to IR happens at run time in pure Rust
(`cljrs_ir::lower`); this crate ships source, not a prebuilt IR bundle.

## Public API

### Entry points

```rust
/// Install every built-in stdlib namespace into a runtime.
pub fn install(runtime: &Runtime);

/// The same, addressed by GlobalEnv, for callers that only hold an
/// environment handle.
pub fn register(globals: &Arc<GlobalEnv>);
```

Both are idempotent: a second call does not re-evaluate sources (`load_ns`'s
already-loaded guard prevents that), but it does overwrite the native fn
registrations in each namespace. Namespaces are registered as embedded
*sources*, so each is parsed and evaluated on its first `require` rather than at
install time.

```rust
use cljrs_runtime::{ExecutionMode, Runtime};

let runtime = Runtime::builder()
    .execution_mode(ExecutionMode::Tiered)
    .source_paths(paths)
    .build()?;
cljrs_stdlib::install(&runtime);
```

Each native module exposes its own registrar, called by `register()`:

```rust
pub fn string::register(globals: &Arc<GlobalEnv>, ns: &str);
pub fn set::register(globals: &Arc<GlobalEnv>, ns: &str);
pub fn io::register(globals: &Arc<GlobalEnv>, ns: &str);   // wasm32: absent
pub fn edn::register(globals: &Arc<GlobalEnv>, ns: &str);  // wasm32: absent
```

`io` is the only one of these that is `pub` at the crate root; it also exports
the `IoReader`, `IoWriter`, and `StringReader` native object types.

### Namespaces provided

| Namespace | Implementation | Notes |
|-----------|---------------|-------|
| `clojure.string` | `string.rs` + `clojure/string.cljrs` | Native Rust, loaded lazily |
| `clojure.set` | `set.rs` + `clojure/set.cljrs` | Native Rust, loaded lazily |
| `clojure.test` | `clojure/test.cljrs` | Pure Clojure, loaded lazily |
| `clojure.spec.alpha` | `clojure/spec/alpha.cljrs` | Pure Clojure, loaded lazily |
| `clojure.spec.test.alpha` | `clojure/spec/test/alpha.cljrs` | Pure Clojure, loaded lazily |
| `clojure.spec.gen.alpha` | `clojure/spec/gen/alpha.cljrs` | Pure Clojure, loaded lazily; every fn throws (no generator engine) |

### clojure.string functions

`upper-case`, `lower-case`, `capitalize`, `trim`, `triml`, `trimr`,
`trim-newline`, `blank?`, `starts-with?`, `ends-with?`, `includes?`,
`replace`, `replace-first`, `split`, `split-lines`, `join`,
`index-of`, `last-index-of`

`replace` and `replace-first` accept either a string or a regex pattern
(`#"..."`) as the match argument. When the match is a pattern, `replace`
replaces all occurrences and `replace-first` replaces only the first.

### clojure.set functions

`union`, `intersection`, `difference`, `subset?`, `superset?`,
`select`, `map-invert`

### clojure.spec.alpha functions and macros

A from-scratch, spec-compatible-in-spirit implementation of the core spec
engine: predicate/set/keyword specs, `and`/`or` composition, `keys`/`merge`,
the derivative-based regex engine (`cat`/`alt`/`*`/`+`/`?`/`&`), collection
specs, and fn-specs (`fdef`/`fspec`).

Macros: `def`, `spec`, `and`, `or`, `keys`, `merge`, `cat`, `alt`, `*`, `+`,
`?`, `&`, `every`, `coll-of`, `every-kv`, `map-of`, `tuple`, `nilable`,
`multi-spec`, `conformer`, `nonconforming`, `fspec`, `fdef`, `assert`

Functions: `registry`, `get-spec`, `invalid`, `invalid?`, `spec-name`,
`spec?`, `conform`, `unform`, `valid?`, `form`, `describe`, `explain-data`,
`explain`, `explain-str`, `explain-out`, `regex?`, `int-in`, `double-in`,
`inst-in` (throws — see deviations), `check-asserts?`, `check-asserts`,
`with-gen`, `gen`, `exercise`, `exercise-fn` (the last three throw — see
deviations)

#### Deviations from JVM clojure.spec.alpha

- No generators: `s/gen`, `s/exercise`, `s/exercise-fn`, and
  `stest/check`/`stest/check-fn` all throw a clear `ex-info` instead of
  running test.check. `s/with-gen` stores the generator fn without ever
  invoking it.
- `s/form` and `s/describe` show forms exactly as written (unqualified) —
  they do not auto-qualify bare symbols to `clojure.core` the way upstream
  does.
- `multi-spec` unform re-dispatches by calling the multimethod on the
  *conformed* value rather than applying JVM spec's `retag` mechanism.
- `check-asserts` takes effect immediately (read at assert-expansion time),
  not just at compile time as on the JVM.
- `inst-in` always throws — this runtime has no `inst?`/date value type to
  bound.
- Users must `:require` the namespace with an alias (`:as s`, the universal
  convention anyway). `:refer`ing names like `and`, `or`, or `def` collides
  with clojurust's special forms, and `:refer-clojure :exclude` is a no-op
  in this runtime, so it cannot be used to work around the collision.

### clojure.spec.test.alpha functions and macros

Macros: `with-instrument-disabled`

Functions: `instrument`, `unstrument`, `instrumentable-syms`, `check` (throws
— see deviations), `check-fn` (throws — see deviations)

`instrument` wraps a var's root fn so every call conforms its argument list
against the fn's `fdef`'d `:args` spec before delegating to the original fn;
only `:args` is checked (never `:ret`/`:fn`, matching upstream). `unstrument`
restores the original fn. Both are idempotent.

### clojure.spec.gen.alpha functions

`generate`, `sample`, `gen-for-pred`, `choose`, `such-that`, `fmap`, `one-of`,
`return`, `elements`, `int`, `string`, `keyword`, `boolean`, `double`,
`simple-type`, `any`

This namespace has no generator engine (no test.check port): every function
throws a clear `ex-info` explaining that generators are not implemented. It
exists purely so that idiomatic specs which `(:require [clojure.spec.gen.alpha
:as gen])` load cleanly. As in upstream, several names (`int`, `string`,
`keyword`, `boolean`, `double`) intentionally shadow `clojure.core`.

## Dependency notes

- `cljrs-stdlib` depends on `cljrs-runtime` (for `Runtime` and `GlobalEnv`)
- `cljrs-runtime` does **not** depend on `cljrs-stdlib` (no circular dep)
- The `cljrs` binary depends on both: it builds the runtime and then calls
  `cljrs_stdlib::install()` so stdlib namespaces are available
