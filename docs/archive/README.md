# Archived implementation plans

These are design and implementation plans whose work has **landed**. They are
kept because they record *why* a subsystem is shaped the way it is — the
alternatives considered, the invariants the implementation has to preserve, and
the phase ordering it was built in. None of them is a to-do list any more.

**Their crate names and file paths are as they were when the work was done.**
The crate consolidation (see [`../crate-consolidation-plan.md`](../crate-consolidation-plan.md))
later merged several of those packages into modules, so paths in these documents
will not resolve against the current tree. Deliberately so: rewriting a finished
plan's paths would make it a worse record of what actually happened. Translate
with this table when following one:

| Plan says | Now |
|---|---|
| `cljrs-env` | `cljrs_runtime::env` |
| `cljrs-builtins` | `cljrs_runtime::builtins` |
| `cljrs-interp` | `cljrs_runtime::interp` |
| `cljrs-eval` | `cljrs_runtime::tiered` |
| `cljrs-jit` | `cljrs_compiler::jit` |
| `cljrs-deps` | `cljrs_project::config` |
| `cljrs-vcs` | `cljrs_project::vcs` |
| `cljrs-dylib` | `cljrs::native::pinned` |
| `cljrs-dom` | `cljrs_wasm::dom` |
| `cljrs-ir-viz` | `cljrs::commands::ir::viz` |
| `cljrs-ir-prebuild` | `cljrs::commands::ir` |
| `cljrs-logging` | `tracing` targets + `cljrs_runtime::logging` |

| Plan | Subject | Where the shipped code lives |
|---|---|---|
| [`async-plan.md`](async-plan.md) | `clojure.core.async`: channels, `go`, `^:async` fns, the `AsyncRuntime` hook | `cljrs-async` |
| [`async-lowering-plan.md`](async-lowering-plan.md) | AOT/JIT state-machine lowering for `^:async` functions (async-plan phase H) | `cljrs-ir`, `cljrs-compiler`, `cljrs-async` |
| [`networking-plan.md`](networking-plan.md) | Channel-oriented TCP/Unix/TLS/UDP transports and framing | `cljrs-net` |
| [`quic-http3-integration-plan.md`](quic-http3-integration-plan.md) | QUIC and HTTP/3 over quinn, on the same channel shape | `cljrs-net` |
| [`jit-plan.md`](jit-plan.md) | Tiered execution: tree walk → IR interpreter → Cranelift native | `cljrs_compiler::jit`, `cljrs_runtime::tiered` |
| [`wasm-aot-plan.md`](wasm-aot-plan.md) | The WebAssembly code-generation backend | `cljrs_compiler::wasm` (`wasm-aot` feature) |
| [`no-gc-plan.md`](no-gc-plan.md) | The `no-gc` build: region allocation and the forbidden-operation blacklist | `cljrs-gc`, `cljrs_ir::lower::escape`, the `no-gc` feature chain |
| [`versioned-namespaces-plan.md`](versioned-namespaces-plan.md) | `my.ns@<commit>` versioned symbols and namespaces | `cljrs_runtime::env::versioned`, `cljrs-project`, `cljrs::native::pinned` |
| [`gitoxide-native-vcs-plan.md`](gitoxide-native-vcs-plan.md) | Replacing the `git` CLI with gitoxide and native signature verification | `cljrs_project::vcs` |
| [`native-ssh-fetch-plan.md`](native-ssh-fetch-plan.md) | Pure-Rust `ssh://` git fetching over russh | `cljrs_project::vcs` (`ssh` feature) |

Plans with open work stay in [`../`](..): `async-worker-pool-plan.md` and
`isolate-boundary-plan.md` (isolates, phases B2–B3),
[`crate-consolidation-plan.md`](../crate-consolidation-plan.md), and
`replicant.md` (a requirements spec for a `cljrs.dom` consumer, not an
implementation plan). `crates/cljrs-ir/ESCAPE_OPT_PLAN.md` also stays: its
stage 3 is unimplemented.
