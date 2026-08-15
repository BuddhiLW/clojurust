# Consolidation Baseline

Stage 0 of [`crate-consolidation-plan.md`](crate-consolidation-plan.md).
These are the pre-consolidation measurements every later stage is compared against.

Measured on the branch point of `claude/crate-consolidation-plan-xhwi32` (commit `436a035`),
Linux x86_64, `rustc` 1.94.1 / `cargo` 1.94.1, `dev` profile.

> **Profile note.** The container this was measured in has a bounded writable disk
> allowance, and a full `dev` build with the repo-default `debug = 2` produces a
> ~30 GiB `target/` that exhausts it during `cargo test --workspace`. All figures
> below are labelled with the profile used: **(D2)** = repo default `debug = 2`,
> **(D0)** = `debug = 0, incremental = false` set in `~/.cargo/config.toml` (a
> machine-local override; the repository is unchanged). Re-measure at Stage 6 with
> the same label to compare like with like.

---

## 1. Workspace size

| Metric | Value |
|---|---|
| Cargo packages (`cargo metadata --no-deps`) | 34 |
| Workspace members in `Cargo.toml` | 34 (33 crates + `examples/rust-interop`) |
| Rust source lines (`crates/`, `examples/`, `tests/`) | 108,049 |
| Clojure source lines (`.cljrs` / `.cljc` / `.clj`) | 18,628 |

### Rust lines per package

| Package | Lines | Package | Lines |
|---|---:|---|---:|
| `cljrs-compiler` | 20,292 | `cljrs-lsp` | 991 |
| `cljrs-builtins` | 10,432 | `cljrs-charset` | 786 |
| `cljrs-interp` | 10,335 | `cljrs-dylib` | 734 |
| `cljrs-ir` | 9,816 | `cljrs-io` | 688 |
| `cljrs-net` | 8,541 | `cljrs-tx` | 626 |
| `cljrs-value` | 8,011 | `cljrs-deps` | 591 |
| `cljrs-eval` | 5,459 | `cljrs-interop` | 556 |
| `cljrs-async` | 4,687 | `cljrs-blake3` | 360 |
| `cljrs` | 4,277 | `rust-interop` (example) | 345 |
| `cljrs-gc` | 3,742 | `cljrs-ir-prebuild` | 342 |
| `cljrs-env` | 3,000 | `cljrs-export-macro` | 311 |
| `cljrs-stdlib` | 2,567 | `cljrs-base64` | 242 |
| `cljrs-reader` | 2,468 | `cljrs-wasm` | 185 |
| `cljrs-jit` | 1,824 | `cljrs-logging` | 158 |
| `cljrs-nrepl` | 1,609 | `cljrs-types` | 86 |
| `cljrs-vcs` | 1,533 | `cljrs-runtime` | 9 |
| `cljrs-dom` | 1,282 | | |
| `cljrs-ir-viz` | 1,164 | | |

`cljrs-runtime` is a nine-line doc-comment stub with no code, as the plan states.

---

## 2. Internal dependency graph

Workspace-internal dependencies only (external crates omitted). Count in parentheses.

```
cljrs (24)              -> async, base64, charset, compiler, deps, dylib, eval, gc, interop,
                           io, ir, ir-prebuild, ir-viz, jit, logging, lsp, net, nrepl,
                           reader, runtime, stdlib, types, value, vcs
cljrs-compiler (16)     -> async, base64, builtins, charset, deps, env, eval, gc, interop,
                           io, ir, net, reader, stdlib, types, value
cljrs-jit (11)          -> async, compiler, env, eval, gc, interp, ir, logging, reader, stdlib, value
cljrs-wasm (10)         -> async, builtins, dom, env, gc, interp, reader, stdlib, types, value
cljrs-dylib (9)         -> deps, env, gc, interop, interp, logging, reader, value, vcs
cljrs-eval (9)          -> builtins, env, gc, interp, ir, logging, reader, types, value
cljrs-stdlib (9)        -> builtins, env, eval, gc, interp, ir, logging, reader, value
cljrs-net (8)           -> async, env, gc, interp, reader, stdlib, types, value
cljrs-async (7)         -> builtins, env, gc, interp, reader, types, value
cljrs-base64 (7)        -> env, eval, gc, interop, reader, stdlib, value
cljrs-blake3 (7)        -> env, eval, gc, interop, reader, stdlib, value
cljrs-env (7)           -> deps, gc, logging, reader, types, value, vcs
cljrs-interp (7)        -> builtins, env, gc, reader, types, value, vcs
cljrs-io (7)            -> async, env, gc, interp, reader, types, value
cljrs-nrepl (7)         -> builtins, env, eval, gc, reader, stdlib, value
cljrs-ir-prebuild (6)   -> env, eval, ir, reader, types, value
cljrs-tx (6)            -> env, gc, interp, reader, types, value
cljrs-builtins (5)      -> env, gc, reader, types, value
cljrs-interop (5)       -> env, export-macro, gc, types, value
cljrs-charset (4)       -> async, env, gc, value
cljrs-dom (4)           -> async, env, gc, value
cljrs-ir-viz (4)        -> compiler, ir, reader, types
cljrs-deps (3)          -> gc, reader, types
cljrs-runtime (3)       -> eval, gc, types
cljrs-value (3)         -> gc, reader, types
cljrs-ir (2)            -> reader, types
cljrs-lsp (2)           -> reader, types
cljrs-gc (1)            -> logging
cljrs-export-macro (0)
cljrs-logging (0)
cljrs-types (0)
cljrs-vcs (0)
```

`cljrs-compiler`'s 16 direct internal dependencies include the optional product
extensions (`async`, `base64`, `charset`, `io`, `net`) that `aot.rs` initializes
unconditionally — the fan-out the plan calls out. Those dependencies are **not**
optional, so every consumer of `cljrs-compiler` compiles the networking and async
stacks whether or not it uses them.

Regenerate with:

```bash
cargo metadata --no-deps --format-version 1 \
  | python3 -c "import json,sys; d=json.load(sys.stdin); ws={p['name'] for p in d['packages']}; [print(p['name'], sorted({x['name'] for x in p['dependencies']} & ws)) for p in sorted(d['packages'], key=lambda x: x['name'])]"
```

---

## 3. Build, startup, and size

| Metric | Profile | Value |
|---|---|---|
| `cargo build --workspace`, cold `target/` | D2 | 2 m 59 s wall / 7 m 28 s user |
| `cargo clippy --workspace -- -D warnings`, warm | D2 | 1 m 21 s wall / 3 m 41 s user |
| `cargo test --workspace`, cold `target/` (build + run) | D0 | 10 m 48 s wall / 27 m 05 s user |
| `target/debug/cljrs` size | D2 | 501,157,720 bytes (478 MiB) |
| `target/debug/cljrs` size | D0 | 115,678,040 bytes (110 MiB) |
| `target/` after a full workspace build | D2 | ~30 GiB |
| `target/` after `cargo test --workspace` | D0 | ~13 GiB |

Cold start of the `cljrs` binary (D2, best of three, wall clock):

| Command | Time |
|---|---|
| `cljrs eval 'nil'` | 0.050 s |
| `cljrs eval '(do (require (quote [clojure.string :as s])) (s/upper-case "hi"))'` | 0.044 s |

Startup does **not** load any prebuilt IR bundle — see §5.

Tiering smoke benchmark (D0 binary), `fib` to 25 summed over `(range 25)`:

| Mode | Time |
|---|---|
| `--jit-threshold 100` (Tier 0 → 1 → 2) | 2.40 s |
| `--jit-threshold 0 --ir-threshold 0` (tree walk only) | 7.23 s |

---

## 4. Validation gate

| Check | Result |
|---|---|
| `cargo fmt --check` | **pass** |
| `cargo clippy --workspace -- -D warnings` | **pass** (no warnings) |
| `cargo build --workspace` | **pass** |
| `cargo test --workspace` | **pass** — 1077 passed, 0 failed, 24 ignored, across 147 suites |
| Clojure test suite (AOT-compiled) | **pass** — 242 suites, 629 tests, 11,005 assertions, 0 failures |
| CLI smoke samples | **pass**, one environment-limited failure (§4.3) |
| `no-gc` builds | **fail** — pre-existing breakage (§4.2) |
| WASM build | **not runnable here** (§4.4) |

### 4.1 Default test matrix

`cargo test --workspace` (D0), aggregated across all 147 test binaries and doc-test
targets:

```
1077 passed; 0 failed; 24 ignored
```

The AOT-compiled Clojure suite (`cljrs compile --test ./clojure-test-suite/test`,
then run the binary) reports:

```
242 test suites, 629 tests, 11005 assertions, 0 failures, 0 errors
```

Compiling that suite takes 3 m 54 s wall / 12 m 57 s user with the D0 `cljrs`
binary; the run reports 230 GC collections, 44.4 s total GC pause, 62,978,473
objects and 7,447,163,716 bytes freed, 0 region (bump) allocations, 0 boundary
crossings.

### 4.2 `no-gc` build — broken at baseline

The plan's later validation gates say "Default and `no-gc` builds pass". They do
not pass today. Recording the true starting state:

| Command | Result |
|---|---|
| `cargo check -p cljrs --features no-gc` | **fail** |
| `cargo check -p cljrs --no-default-features --features no-gc` | **fail** |
| `cargo check -p cljrs-gc --features no-gc` | pass |
| `cargo check -p cljrs-value --features no-gc` | pass |
| `cargo check -p cljrs-interp --features no-gc` | pass |
| `cargo check -p cljrs-eval --features no-gc` | pass |
| `cargo check -p cljrs-tx --features no-gc` | pass |
| `cargo check -p cljrs-env --features no-gc` | **fail** |
| `cargo check -p cljrs-builtins --features no-gc` | **fail** |
| `cargo check -p cljrs-stdlib --features no-gc` | **fail** |
| `cargo check -p cljrs-async --features no-gc` | **fail** — the feature does not exist |

Three distinct defects:

1. **`cljrs-async` has no `no-gc` feature at all**, yet `crates/cljrs-async/src/isolate.rs:44`
   calls `cljrs_gc::HEAP.set_config_from_env()`, which only exists in the GC build.
   Because `cljrs-compiler` depends on `cljrs-async` unconditionally, *any* `no-gc`
   build that reaches the compiler fails here — including the CLI with
   `--no-default-features`.
2. **`cljrs-stdlib`'s `no-gc` feature does not forward to `cljrs-env`**, so
   `standard_env()` still calls `HEAP.set_config_from_env()` against the no-gc
   `GcHeap`.
3. **`cljrs-env` and `cljrs-builtins` `no-gc` features do not forward
   `cljrs-value/no-gc`**, so `GcPtr::is_region_alloc` is missing when they are
   checked on their own. They compile only when a downstream crate (e.g.
   `cljrs-interp`) unifies the features for them.

Defect 1 is exactly the coupling Stage 4 removes ("A compiler build without network
extensions does not compile `cljrs-net`"); defects 2 and 3 are feature-wiring bugs
that the Stage 2/3 runtime merge should collapse. **These are pre-existing and must
not be counted as consolidation regressions**, but the `no-gc` gate cannot be
treated as green until they are fixed.

### 4.3 CLI smoke tests

Mirrors the `ci.yml` job steps, run with the D0 `cljrs` binary.

| Step | Result |
|---|---|
| `compile --test ./clojure-test-suite/test` + run | pass (see §4.1) |
| `compile -o graph-sample samples/graph.cljrs` + run | pass |
| `compile -o life-sample samples/life.cljrs` + run | pass |
| `compile -o async-sample samples/core_async.cljrs` + run | pass |
| `compile -o http-get-sample samples/http_get.cljrs` + run | compiles and runs; **network step fails in this container** |

The `http_get` sample compiles and executes, then fails its TLS handshake with
`invalid peer certificate: UnknownIssuer`. That is the sandbox's intercepting HTTPS
proxy, not a code defect — the plain-HTTP request in the same sample succeeds
(`HTTP/1.1 426 Upgrade Required` from the server). Treat this step as
environment-limited, and rely on CI for it.

### 4.4 WASM

`wasm32-unknown-unknown` is not installed in this container and `wasm-pack` is not
available, so `wasm-pack build crates/cljrs-wasm` cannot be run here. The `wasm` CI
job remains the gate for Stage 5's `cljrs-dom` → `cljrs-wasm::dom` move.

---

## 5. IR bundle feature: evidence for the Stage 1 decision

- `cljrs_eval::load_prebuilt_ir` is defined in `crates/cljrs-eval/src/lib.rs:100`
  and is called from **nowhere** in the workspace. No runtime path loads a bundle.
- `cljrs-stdlib/build.rs` does not build IR; it only emits
  `cargo::rerun-if-changed=build.rs`. There is no `core_ir.bin`, no `prebuild-ir`
  feature, and no `include_bytes!` of a bundle anywhere in the tree — despite what
  the root README and the `cljrs-stdlib` README still describe.
- The standalone `cljrs-ir-prebuild` binary and the `cljrs ir build` / `cljrs ir dump`
  subcommands call the same `run_prebuild` entry point.
- Cold start is already ~50 ms with no bundle loaded, so there is no measured
  cold-start benefit to preserve.

Decision recorded in the plan: keep `cljrs ir build` / `cljrs ir dump` as CLI
diagnostics, move `run_prebuild` into the CLI, and delete the duplicate
`cljrs-ir-prebuild` package and its standalone binary.

---

## 6. Baseline package inventory

```
cljrs                cljrs-async          cljrs-base64         cljrs-blake3
cljrs-builtins       cljrs-charset        cljrs-compiler       cljrs-deps
cljrs-dom            cljrs-dylib          cljrs-env            cljrs-eval
cljrs-export-macro   cljrs-gc             cljrs-interop        cljrs-interp
cljrs-io             cljrs-ir             cljrs-ir-prebuild    cljrs-ir-viz
cljrs-jit            cljrs-logging        cljrs-lsp            cljrs-net
cljrs-nrepl          cljrs-reader         cljrs-runtime        cljrs-stdlib
cljrs-tx             cljrs-types          cljrs-value          cljrs-vcs
cljrs-wasm           rust-interop-example
```

---

## 7. Stage 6 re-measurement

Item 6 of Stage 6: regenerate the dependency graph and the baseline measurements.
Same machine, same profile labels as §1–§3 so the two are comparable.

### 7.1 Workspace size

| Metric | Baseline | Now | Δ |
|---|---:|---:|---:|
| Cargo packages (`cargo metadata --no-deps`) | 34 | 23 | −11 |
| Workspace members in `Cargo.toml` | 34 | 23 (22 crates + `examples/rust-interop`) | −11 |
| Rust source lines (`crates/`, `examples/`, `tests/`) | 108,049 | 111,491 | +3,442 |
| Clojure source lines (`.cljrs` / `.cljc` / `.clj`) | 18,628 | 18,628 | 0 |

The plan's target was "approximately 23 packages"; the workspace is at 23.

Rust lines rise slightly. Consolidation removed manifests and re-export shims,
not code, and the stages added real code as they went: the `Runtime` builder and
`TierState` (Stage 3), per-runtime `JitState`/`Tiers` with its `Drop`-time code
reclamation and the `extensions` registry (Stage 4), `cljrs_runtime::logging`
and the CLI command modules (Stage 5), plus the tests for all of it.

#### Rust lines per package

| Package | Lines | Package | Lines |
|---|---:|---|---:|
| `cljrs-runtime` | 31,026 | `cljrs-nrepl` | 1,621 |
| `cljrs-compiler` | 22,967 | `cljrs-wasm` | 1,477 |
| `cljrs-ir` | 9,832 | `cljrs-lsp` | 991 |
| `cljrs-net` | 8,615 | `cljrs-charset` | 790 |
| `cljrs-value` | 8,011 | `cljrs-io` | 697 |
| `cljrs` | 6,541 | `cljrs-tx` | 581 |
| `cljrs-async` | 4,721 | `cljrs-interop` | 556 |
| `cljrs-gc` | 3,747 | `cljrs-blake3` | 359 |
| `cljrs-reader` | 3,318 | `rust-interop` (example) | 350 |
| `cljrs-stdlib` | 2,513 | `cljrs-export-macro` | 311 |
| `cljrs-project` | 2,140 | `cljrs-base64` | 241 |
| | | `cljrs-types` | 86 |

`cljrs-runtime` was a nine-line stub at baseline. It is now the merge of
`cljrs-env` + `cljrs-builtins` + `cljrs-interp` + `cljrs-eval` (29,226 baseline
lines between them) plus the builder, tier state, and logging filter.

### 7.2 Internal dependency graph

Normal (non-dev) workspace-internal dependencies. Count in parentheses;
dev-only internal dependencies in brackets.

```
cljrs (17)               -> async, base64, charset, compiler, gc, interop, io, ir, lsp,
                            net, nrepl, project, reader, runtime, stdlib, types, value
cljrs-compiler (9)       -> async, gc, ir, project, reader, runtime, stdlib, types, value
                            [dev: base64, charset, io, net]
cljrs-wasm (7)           -> async, gc, reader, runtime, stdlib, types, value
cljrs-io (6)             -> async, gc, reader, runtime, types, value
cljrs-net (6)            -> async, gc, reader, runtime, types, value   [dev: stdlib]
cljrs-runtime (6)        -> gc, ir, project, reader, types, value
rust-interop-example (6) -> gc, interop, reader, runtime, stdlib, value
cljrs-async (5)          -> gc, reader, runtime, types, value
cljrs-interop (5)        -> export-macro, gc, runtime, types, value
cljrs-stdlib (5)         -> gc, ir, reader, runtime, value
cljrs-tx (5)             -> gc, reader, runtime, types, value
cljrs-base64 (4)         -> gc, interop, runtime, value   [dev: reader, stdlib]
cljrs-charset (4)        -> async, gc, runtime, value
cljrs-nrepl (4)          -> gc, reader, runtime, value   [dev: stdlib]
cljrs-blake3 (3)         -> gc, interop, value   [dev: reader, runtime, stdlib]
cljrs-project (3)        -> gc, reader, types
cljrs-value (3)          -> gc, reader, types
cljrs-ir (2)             -> reader, types
cljrs-lsp (2)            -> reader, types
cljrs-reader (1)         -> types
cljrs-export-macro (0)
cljrs-gc (0)
cljrs-types (0)
```

Regenerate with:

```bash
cargo metadata --no-deps --format-version 1 | python3 -c "
import json,sys
d=json.load(sys.stdin); ws={p['name'] for p in d['packages']}
for p in sorted(d['packages'], key=lambda x: x['name']):
    normal=sorted({x['name'] for x in p['dependencies'] if x['kind'] is None} & ws)
    dev=sorted({x['name'] for x in p['dependencies'] if x['kind']=='dev'} & ws)
    print(p['name'], len(normal), normal, '[dev:', dev, ']' if dev else '')"
```

Against the baseline graph:

- **`cljrs-compiler`: 16 → 9 direct internal dependencies.** The plan's Stage 6
  gate asks that it depend "mainly on `cljrs-runtime`, `cljrs-ir`, and
  `cljrs-project`" — those three plus the `types`/`gc`/`value`/`reader` core it
  generates code against, `cljrs-stdlib` for the bootstrap environment macro
  expansion needs, and `cljrs-async` for the state-machine poll ABI its codegen
  implements. The four product extensions it used to initialize directly
  (`base64`, `charset`, `io`, `net`) are dev-dependencies: the end-to-end tests
  supply them the way a host does.
- **`cljrs`: 24 → 17.** It dropped `deps`, `dylib`, `eval`, `ir-prebuild`,
  `ir-viz`, `jit`, `logging`, and `vcs`, and gained `project`. Every remaining
  entry is a product package or a core type layer; none is an internal
  execution module.
- **`cljrs-gc (1) -> logging` is now `cljrs-gc (0)`.** The GC's diagnostics are
  ordinary `tracing` targets, so the bottom of the graph has no internal
  dependency at all.
- **`cljrs-runtime (3) -> eval, gc, types` is now `(6) -> gc, ir, project,
  reader, types, value`.** At baseline it was a stub that depended on the
  evaluator; it is now the package the evaluator is part of.

### 7.3 Baseline defects

All three `no-gc` defects recorded in §4.2 are fixed — see the Stage 2, 3, and 4
outcomes in the plan for which stage closed which. `no-gc` builds green across
every package that carries the feature, and for `cljrs` with and without default
features.

### 7.4 Package inventory

```
cljrs                cljrs-async          cljrs-base64         cljrs-blake3
cljrs-charset        cljrs-compiler       cljrs-export-macro   cljrs-gc
cljrs-interop        cljrs-io             cljrs-ir             cljrs-lsp
cljrs-net            cljrs-nrepl          cljrs-project        cljrs-reader
cljrs-runtime        cljrs-stdlib         cljrs-tx             cljrs-types
cljrs-value          cljrs-wasm           rust-interop-example
```

Removed across stages 1–6 (11 packages): `cljrs-builtins`, `cljrs-deps`,
`cljrs-dom`, `cljrs-dylib`, `cljrs-env`, `cljrs-eval`, `cljrs-interp`,
`cljrs-ir-prebuild`, `cljrs-ir-viz`, `cljrs-jit`, `cljrs-logging`, `cljrs-vcs`
— twelve names, eleven net, because `cljrs-project` was created from two of them.
