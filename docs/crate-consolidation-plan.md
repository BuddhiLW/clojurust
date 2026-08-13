# Crate Consolidation Plan

## Purpose

This plan reduces workspace package count and removes obsolete architecture.
It keeps package boundaries that isolate a real artifact, platform, or optional dependency.

The workspace contains 34 packages and approximately 111,000 lines of Rust and Clojure source.
The proposed target contains approximately 23 packages.

This plan does not remove the tree-walking interpreter, JIT, AOT compiler, no-GC mode, or supported platform targets.

## Progress

| Stage | Status | Notes |
|---|---|---|
| 0. Record the baseline | Complete | [`consolidation-baseline.md`](consolidation-baseline.md) |
| 1. Remove obsolete debris | Complete | `cljrs-ir-prebuild` folded into `cljrs::commands::ir`; 34 packages → 33 |
| 2. Create the merged runtime | Not started | |
| 3. Simplify runtime state | Not started | |
| 4. Merge JIT and compiler packages | Not started | |
| 5. Consolidate project and CLI tools | Not started | |
| 6. Remove compatibility packages | Not started | |

Package count: 34 at baseline, 33 now, approximately 23 at target.

### Corrections found while measuring the baseline

Three Evidence items were already resolved before this work started, so they need
no Stage 1 change:

- `cljrs-stdlib/src/core_async.rs` does not exist. The only `core_async` source is
  `crates/cljrs-async/src/core_async.cljrs`, which is live Clojure source, not
  commented-out Rust.
- `cljrs-stdlib/Cargo.toml` no longer lists `tokio` or `lazy_static`. Only
  `Cargo.lock` still carried the stale entries.
- No Clojure compiler namespaces remain, so `cljrs-ir-prebuild` no longer loads
  any — but its own docs still claim it does, which Stage 1 corrects.

The baseline also found one thing the plan did not anticipate: **`no-gc` does not
build today**, for the CLI or for `cljrs-env`, `cljrs-builtins`, `cljrs-stdlib`,
and `cljrs-async` on their own. See
[`consolidation-baseline.md` §4.2](consolidation-baseline.md). Later stages state
"Default and `no-gc` builds pass" as a gate; until those pre-existing defects are
fixed, `no-gc` can only be held to "no worse than baseline".

## Background

The current core split came from an earlier compiler bootstrap design.
That design ran compiler code written in Clojure before it lowered functions to IR.

The compiler front end now uses Rust code in `cljrs-ir` and `cljrs-eval`.
No Clojure compiler namespaces remain in the repository.
The old split now adds callbacks, duplicate constructors, and broad dependency chains.

The main overlap is between these packages:

- `cljrs-env`
- `cljrs-builtins`
- `cljrs-interp`
- `cljrs-eval`
- `cljrs-runtime`
- `cljrs-stdlib`
- `cljrs-ir`
- `cljrs-compiler`
- `cljrs-jit`

## Evidence

### Obsolete packages and modules

`cljrs-runtime` is an unused nine-line stub.
The CLI depends on it only to propagate the `no-gc` feature.

`cljrs-compiler::ir` only re-exports `cljrs-ir`.
This module does not define a compiler boundary.

`cljrs-stdlib/build.rs` does not build IR.
It only emits a Cargo rerun directive.
The root README still describes an obsolete build-time compiler flow.

`cljrs-stdlib/src/core_async.rs` contains only commented-out code.
It retains unused `tokio` and `lazy_static` dependencies.

### Duplicate runtime entry points

`cljrs-interp`, `cljrs-eval`, and `cljrs-stdlib` each construct a standard environment.
These constructors represent modes of one runtime, not separate products.

`cljrs-eval` also re-exports environment and interpreter APIs.
This facade hides package ownership and increases direct dependencies in downstream packages.

### Callback seams

`GlobalEnv` stores function pointers for form evaluation, function calls, and function-definition hooks.
These pointers break dependency cycles between runtime packages.

The pointers are not extension APIs for users.
The runtime can use direct module calls after the packages merge.

### Compiler dependency fan-out

`cljrs-compiler` directly depends on 16 internal packages.
It also initializes async, I/O, networking, charset, and Base64 support during AOT compilation.

The compiler backend must not select product extensions.
The CLI or an embedding host must supply the required extension set.

### IR bundle tooling

`cljrs-ir-prebuild` duplicates the `cljrs ir build` and `cljrs ir dump` commands.
The standard runtime does not load the generated bundles.

The package documentation still says that the tool loads Clojure compiler namespaces.
That statement is no longer correct.

## Boundary rules

When at least one rule applies, keep a package:

1. Cargo requires a separate artifact, such as a procedural macro or `cdylib`.
2. The package isolates a large optional dependency or platform.
3. Multiple independent consumers use a stable low-level API.
4. The boundary prevents a dependency cycle without callback indirection.
5. The package has an independent release or embedding use case.

When none of these rules apply, use a Rust module.

## Target workspace

### Core packages

| Package | Decision | Responsibility |
|---|---|---|
| `cljrs-types` | Keep | Source spans and shared error types. |
| `cljrs-reader` | Keep | Lexer, parser, and `Form` AST. |
| `cljrs-gc` | Keep | Garbage collection, regions, and allocation contexts. |
| `cljrs-value` | Keep | Runtime values, collections, callable values, and resources. |
| `cljrs-runtime` | Replace stub and expand | Environment, builtins, tree walker, tiered evaluation, and runtime construction. |
| `cljrs-stdlib` | Keep | Embedded standard-library sources and native namespace helpers. |
| `cljrs-ir` | Keep | IR model, Rust lowering, optimization, OSR transforms, and serialization. |
| `cljrs-compiler` | Expand | Shared code generation, JIT, native AOT, and optional WASM AOT. |

### Interop and extension packages

| Package | Decision | Responsibility |
|---|---|---|
| `cljrs-interop` | Keep | Rust value conversion, native registration, and export support. |
| `cljrs-export-macro` | Keep | Procedural macro for exported Rust functions. |
| `cljrs-async` | Keep | Async runtime, channels, isolates, and worker pools. |
| `cljrs-io` | Keep | Async file I/O. |
| `cljrs-net` | Keep | Network transports and protocols. |
| `cljrs-charset` | Keep | Charset codecs and stream adapters. |

### Tools and artifacts

| Package | Decision | Responsibility |
|---|---|---|
| `cljrs` | Keep | Main CLI and internal command modules. |
| `cljrs-lsp` | Keep | Standalone LSP server and reusable backend. |
| `cljrs-nrepl` | Keep | nREPL server. |
| `cljrs-wasm` | Keep | Browser artifact and WASM runtime facade. |
| `cljrs-tx` | Keep | Isolated no-GC transaction execution. |
| `cljrs-project` | Create | Project configuration, dependency resolution, and VCS operations. |

### Consolidation map

| Current package | Target location |
|---|---|
| `cljrs-env` | `cljrs-runtime::env` |
| `cljrs-builtins` | `cljrs-runtime::builtins` |
| `cljrs-interp` | `cljrs-runtime::interp` |
| `cljrs-eval` | `cljrs-runtime::tiered` |
| `cljrs-jit` | `cljrs-compiler::jit` |
| `cljrs-ir-viz` | Internal `cljrs::commands::ir` module |
| `cljrs-ir-prebuild` | Internal CLI module or removal |
| `cljrs-deps` | `cljrs-project::config` |
| `cljrs-vcs` | `cljrs-project::vcs` |
| `cljrs-dylib` | Internal CLI native-package module |
| `cljrs-dom` | `cljrs-wasm::dom` |
| `cljrs-logging` | Standard `tracing` targets and filters |

Keep `cljrs-base64` and `cljrs-blake3` as native-package examples.
If no core package requires them, exclude them from default workspace builds.

## Runtime API

The merged runtime owns environment construction and execution-mode selection.
Downstream packages must not assemble runtime internals directly.

The target API has one construction path:

```rust
let runtime = Runtime::builder()
    .execution_mode(ExecutionMode::Tiered)
    .source_paths(paths)
    .gc_config(config)
    .build()?;

cljrs_stdlib::install(&runtime)?;
```

`ExecutionMode` replaces the current constructor variants:

- `TreeWalk`
- `Tiered`
- `TieredNoJit`
- `NoGcTransaction`

The builder also owns source paths, GC configuration, extension registration, and embedded namespace sources.

## Compiler API

The merged compiler contains common code generation and JIT state.
The JIT and AOT paths use the same type inference and runtime ABI definitions.

The compiler accepts a prepared compile session.
The session contains the runtime, source paths, target, and extension descriptors.

The CLI selects extensions from enabled features and project configuration.
The compiler does not contain direct calls to each extension package.

Generated AOT harnesses use generic initialization descriptors.
Optional extensions register their runtime ABI symbols through one registry.

## Work plan

Each stage must remain independently reviewable.
Each stage must pass its validation gate before the next stage starts.

### Stage 0: Record the baseline

1. Record the workspace package count and dependency graph.
2. Record build time, cold-start time, and CLI binary size.
3. Record the full test matrix for default and `no-gc` builds.
4. Record successful JIT, native AOT, transaction, and WASM examples.

Store the measurements in `docs/consolidation-baseline.md`.

Validation gate:

- `cargo build`
- `cargo test`
- `cargo clippy -- -D warnings`
- `cargo fmt --check`
- Existing CLI smoke tests

### Stage 1: Remove obsolete debris

1. Remove the unused `cljrs-runtime` dependency from the CLI.
2. Remove the commented `cljrs-stdlib/src/core_async.rs` module.
3. Remove the unused `tokio` and `lazy_static` dependencies from `cljrs-stdlib`.
4. Remove the empty `cljrs-stdlib/build.rs` script.
5. Replace `cljrs-compiler::ir` imports with direct `cljrs_ir` imports.
6. Remove the re-export module.
7. Correct stale README and crate documentation.

Decide the IR bundle feature during this stage.
If baseline measurements show a required cold-start benefit, keep the feature.

If the feature remains, move its implementation into the CLI.
If the feature does not remain, remove its package, commands, and unused runtime API.

Validation gate:

- The default CLI has unchanged behavior.
- The workspace contains no dead build script or commented implementation module.
- Documentation describes only the Rust lowering path.

#### Stage 1 outcome

Items 2 and 3 needed no source change: `cljrs-stdlib/src/core_async.rs` does not
exist and `cljrs-stdlib/Cargo.toml` no longer lists `tokio` or `lazy_static`.
Only `Cargo.lock` still carried those entries; the Stage 0 commit refreshed it.

What changed:

1. Dropped `cljrs-runtime` from the CLI's dependencies and from its `no-gc`
   feature list. Nothing in the workspace depends on the stub now.
4. Deleted `crates/cljrs-stdlib/build.rs`.
5. and 6. Deleted `crates/cljrs-compiler/src/ir.rs` and rewrote all 27
   `crate::ir::` paths inside `cljrs-compiler` to `cljrs_ir::`. No package
   outside `cljrs-compiler` referenced `cljrs_compiler::ir`.
7. Corrected the root README's "Prebuilt IR pipeline" section (it described a
   build-time bootstrap through Clojure compiler namespaces that does not
   exist), its dependency graph, its crate table and repository layout, the
   `cljrs-stdlib` README's `build.rs` entry and incomplete file/API lists, the
   `cljrs-compiler` README's `ir.rs` entry, the `cljrs` README's dependency
   table, the `cljrs-eval` README's `load_prebuilt_ir` note, the
   `cljrs-value` README's forward reference to `cljrs-runtime`, and the
   `cljrs-runtime` README (which described a `clojure.core` implementation that
   actually lives in `cljrs-builtins` and `cljrs-interp`).

**IR bundle decision: keep the commands, delete the package.** Baseline §5 found
no cold-start benefit to preserve - no runtime path calls `load_prebuilt_ir`,
and cold start is already ~50 ms with no bundle loaded. But `ir build` and
`ir dump` are useful lowerer diagnostics and the public replay API matters for
targets with no background lowering worker, so the feature stays. `run_prebuild`
moved into `cljrs::commands::ir` and the `cljrs-ir-prebuild` package - library,
duplicate standalone binary, and all - is gone. `cljrs_eval::load_prebuilt_ir`
is retained, since the feature it serves remains.

This also creates `crates/cljrs/src/commands/`, the module tree Stage 5 splits
`main.rs` into. `IrCommands`, its dispatch, and `ir viz` moved there with the
prebuild code, so Stage 5 inherits a finished `commands::ir`.

Gate results:

| Check | Result |
|---|---|
| `cargo fmt --check` | pass |
| `cargo clippy --workspace -- -D warnings` | pass |
| `cargo test --workspace` | 1077 passed, 0 failed, 24 ignored - identical to baseline (144 suites vs 147: the three `cljrs-ir-prebuild` targets are gone, no test was lost) |
| Clojure test suite (AOT) | 242 suites, 629 tests, 11,005 assertions, 0 failures |
| `cljrs --help`, `cljrs ir --help`, `ir dump/viz --help` | byte-identical to the baseline binary |
| `cljrs ir build --help` | one intended text change: it no longer claims bundles are "loaded back at startup" |
| `cljrs ir build --ns clojure.core` | 151 functions lowered, 2 unsupported - identical to baseline. Bundle bytes differ run-to-run in *both* binaries (the bundle is a `HashMap`), and `ir dump` of one bundle is identical modulo ordering |
| `graph`, `life`, `core_async` samples | compile and run |
| `no-gc` | unchanged from baseline: same four packages fail with the same three defects |

Package count: 34 → 33.

### Stage 2: Create the merged runtime

1. Replace the `cljrs-runtime` stub with the new runtime package.
2. Move `cljrs-env` source files into `cljrs-runtime::env`.
3. Move `cljrs-builtins` source files into `cljrs-runtime::builtins`.
4. Move `cljrs-interp` source files into `cljrs-runtime::interp`.
5. Move `cljrs-eval` source files into `cljrs-runtime::tiered`.
6. Add temporary re-export packages for downstream migration.
7. Move tests with their source modules.

Do not change execution behavior during file movement.
Use mechanical import changes and small commits.

Validation gate:

- Tree-walk-only evaluation passes its current tests.
- Tiered evaluation passes its current tests.
- Default and `no-gc` builds pass.
- Existing public paths work through temporary re-exports.

### Stage 3: Simplify runtime state

1. Add `Runtime`, `RuntimeBuilder`, and `ExecutionMode`.
2. Move standard-environment construction into the builder.
3. Change `cljrs-stdlib` to expose `install(&Runtime)`.
4. Remove duplicate standard-environment constructors.
5. Remove evaluator function pointers from `GlobalEnv`.
6. Remove the function-definition hook from `GlobalEnv`.
7. Replace `compiler_ready` with explicit tier state.
8. Move IR caches and tier counters into the runtime instance.
9. Remove cache keys based on the address of `GlobalEnv`.

Keep explicit hooks only for optional systems.
Examples include the async runtime and native-package loaders.

Validation gate:

- One builder creates every supported runtime mode.
- One function-call path selects tree walk, IR, or JIT execution.
- Two runtime instances do not share IR caches or tier counters.
- The runtime has no callback that exists only to break an old package cycle.

### Stage 4: Merge JIT and compiler packages

1. Move `cljrs-jit` into `cljrs-compiler::jit`.
2. Share type inference directly between JIT and AOT code generation.
3. Share code-cache and runtime-ABI definitions inside the compiler.
4. Replace global JIT hooks with compiler state owned by the runtime.
5. Add an extension registry for AOT harness generation.
6. Remove hard-coded extension initialization from `aot.rs`.
7. Put the WASM backend behind a `wasm-aot` feature.
8. Remove the `cljrs-jit` compatibility package after downstream migration.

Validation gate:

- JIT promotion and deoptimization tests pass.
- Native AOT end-to-end tests pass.
- A compiler build without network extensions does not compile `cljrs-net`.
- The compiler does not select optional product extensions.

### Stage 5: Consolidate project and CLI tools

1. Create `cljrs-project` from `cljrs-deps` and `cljrs-vcs`.
2. Move dynamic native-package loading into the CLI.
3. Move IR visualization into `cljrs::commands::ir`.
4. Move retained IR bundle code into the same command module.
5. Move `cljrs-dom` into `cljrs-wasm::dom`.
6. Replace `cljrs-logging` with `tracing` targets and filters.
7. Split `cljrs/src/main.rs` into command modules.

Keep the `cljrs-lsp` and `cljrs-nrepl` packages.
They produce distinct tools and have reusable server APIs.

Validation gate:

- The CLI depends on product-level packages instead of internal runtime layers.
- Project configuration and VCS operations use one package.
- Each standalone artifact still builds independently.
- CLI help and command behavior remain compatible.

### Stage 6: Remove compatibility packages

1. Update all downstream imports to the target packages.
2. Remove temporary re-export packages.
3. Remove unused workspace dependencies and feature propagation.
4. Update the root README and every affected crate README.
5. Archive or remove obsolete implementation plans.
6. Regenerate the dependency graph and baseline measurements.

Validation gate:

- The workspace contains approximately 23 packages.
- `cljrs-compiler` depends mainly on `cljrs-runtime`, `cljrs-ir`, and `cljrs-project`.
- The CLI does not depend directly on internal execution modules.
- All required build and test configurations pass.

## Risks

### GC roots across moved modules

The package merge changes paths but must not change root lifetimes.
Move root-tracing tests before any state redesign.

### Global state leaks between runtimes

Current IR and JIT tables contain process-global state.
Move this state into `Runtime` before removal of compatibility packages.

### JIT code reclamation

The JIT uses hooks for stale code, active epochs, and pending exceptions.
Preserve these invariants during the move into the compiler package.

### Optional extension behavior

AOT currently initializes several extensions automatically.
An extension registry must preserve default CLI behavior and custom embedding behavior.

### Public Rust API changes

The project has version `0.1.0`, but examples and external users can use current package paths.
Keep re-export packages for one migration stage and document replacement paths.

### Large review size

Package moves can hide behavior changes.
Separate mechanical moves from API redesign and state redesign.

## Completion criteria

When all criteria are true, the cleanup is complete:

- The workspace contains approximately 23 packages.
- Every remaining package satisfies at least one boundary rule.
- The runtime has one builder and one execution dispatch path.
- Runtime instances own their tier and cache state.
- The compiler does not initialize optional product extensions directly.
- JIT and AOT code generation share one compiler package.
- The CLI imports product APIs instead of internal layers.
- Default, `no-gc`, JIT, AOT, transaction, LSP, nREPL, and WASM validation passes.
- All crate READMEs describe their current files and public APIs.

## Recommended first pull request

Implement Stage 1 only.
This stage removes obsolete files and dependencies.
It does not change the runtime architecture.

The pull request must include corrected documentation and a new dependency graph.
The runtime merge starts only after this pull request passes the full validation gate.
