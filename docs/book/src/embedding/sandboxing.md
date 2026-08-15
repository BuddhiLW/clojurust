# Limits & sandboxing

An embedded interpreter runs code that came from somewhere else. How much you
need to bound it depends on where that code came from — a config file in your
own repository is not the same problem as a rule a customer typed into a form.

This page covers the three resource bounds a host can apply to an ordinary
runtime, then the capability question, which is where the honest answer matters
most: **an ordinary clojurust runtime is not a security sandbox.** Bounding it
takes a different execution profile.

## Memory

Each thread's heap is bounded by a [`GcConfig`](runtime-builder.md#gc_config-and-gc_config_from_env):
a soft limit that triggers collection and a hard limit that forces one.

```rust
use cljrs_gc::GcConfig;

Runtime::builder()
    .gc_config_from_env(false)                    // operators can't widen it
    .gc_config(Arc::new(GcConfig::with_limits(32 << 20, 64 << 20)))
    .build()?
```

This bounds the *managed* heap — GC-allocated Clojure values. It is not an RSS
ceiling: ordinary Rust allocations made by native functions are not charged
against it. If you need a hard address-space limit against adversarial code, it
has to come from outside the process (a container limit, `setrlimit`, a WASM
linear-memory bound).

## CPU: gas metering

A gas meter is a cooperative execution-credit budget. Every tier charges
against it — the tree walker at each evaluation step, the IR interpreter per
basic block, and JIT-compiled native code through the runtime ABI — so a hot
loop cannot outrun its budget by getting compiled.

```rust
use cljrs_runtime::env::gas::{GasGuard, GasMeter};

let meter = GasMeter::new(10_000);
let guard = GasGuard::install(meter.clone());
let result = eval_str(&mut env, untrusted_src, "<guest>");
drop(guard);

match result {
    Err(EvalError::GasExhausted) => { /* over budget — meter.remaining() == 0-ish */ }
    other => other?,
}
```

`cljrs_runtime::tiered::eval_with_gas(&form, &mut env, credits)` is the
one-liner version: it installs a fresh meter for one form and drops it
afterwards.

Properties worth knowing:

- **Installation is thread-local and scoped.** The guard uninstalls on drop,
  including on unwind.
- **Nested meters are charged together**, so an inner evaluation cannot escape an
  outer budget by installing a smaller one of its own.
- **Exhaustion does not poison an outer scope.** An inner budget running out
  leaves the enclosing evaluation able to continue.
- **Async tasks carry the meter with them** — compiled state machines capture the
  active meter stack when spawned and reinstall it when polled.
- **Unmetered code is free.** With no meter installed, `charge` always succeeds;
  metering costs a thread-local check.

Because charging is per evaluation step rather than per wall-clock tick, a
budget is reproducible across machines — but it is not a timeout. A native
function that blocks (a slow syscall in a host-provided builtin) consumes no
gas. Keep host builtins non-blocking, or enforce time separately.

## Stack: the call-depth cap

Interpreted recursion consumes real Rust stack, and an overflow aborts the
process rather than raising an error your host can catch. A runtime built in
[`ExecutionMode::NoGcTransaction`](execution-modes.md#nogctransaction-and-the-depth-cap)
routes calls through a depth-capped path:

```rust
use cljrs_runtime::env::depth::DepthGuard;

let _depth = DepthGuard::install(1_000);
```

Exceeding it returns `EvalError::Runtime` with the depth-exceeded message. Size
the evaluating thread's stack to comfortably cover the cap you set —
interpreted frames can run to a few kilobytes each. (`cljrs --stack-size-mb` is
the CLI's version of that decision; a host does it with
`std::thread::Builder::stack_size`.)

## Capabilities: what guest code can reach

`clojure.core` — which every runtime has, before any extension is installed —
includes `slurp`, `spit`, `println`, `rand`, clock access, and `new` for Rust
object construction. So:

- Leaving `source_paths` empty does not prevent file access.
- Skipping `cljrs_stdlib::install` does not prevent file access either. It only
  removes `clojure.string`, `clojure.set`, `clojure.edn`, `clojure.rust.io`, and
  friends.
- Not installing `cljrs-net`/`cljrs-io` *does* keep sockets and async file I/O
  out of the binary — extensions are genuinely opt-in, and that is a real
  reduction in reachable surface.

For code you wrote or reviewed, that is usually fine: resource limits are the
thing you actually want, and the guest is not adversarial. For code you did not
write, it is not enough.

## Running untrusted code

The isolation boundary that does exist is the **transaction profile**: a fresh
environment, in a bounded arena, tree-walking only, under a capability denylist,
with pure data crossing in and out. `cljrs-tx` packages it:

```rust
use cljrs_tx::{TxLimits, TxProgram, execute};

let program = TxProgram::parse("(fn [x] (* x 2))")?;
let result = execute(
    &program,
    vec![arg],                                   // SerializedValue in
    TxLimits { memory_bytes: 8 << 20, gas: 100_000, call_depth: 256 },
)?;                                              // SerializedValue out
```

`execute` builds and bootstraps a new interpreter environment inside one bounded
arena, structure-clones the arguments in, invokes the function under a gas meter
and the transaction capability policy, clones a pure-data result out, and
destroys the whole environment. Nothing survives the call.

The policy that makes it a boundary is `TransactionPolicyGuard`, in
`cljrs_runtime::env::policy`. While it is active, a denylist is enforced at the
final call boundary of every native builtin and special form:

| Denied | Examples |
|---|---|
| Filesystem | `slurp`, `spit`, `close` |
| Output | `print`, `println`, `pr`, `prn`, `printf`, `newline`, `flush` |
| Clocks & randomness | `nanotime`, `sleep`, `rand`, `rand-int`, `random-sample`, `shuffle`, `random-uuid` |
| Process-global state | `gensym`, `add-tap`, `remove-tap`, `tap>`, `shared-atom` |
| Concurrency | `promise`, `deliver`, `send`, `send-off` |
| Rust interop | `new`, `Exception.` |
| Loading & namespaces | `.`, `ns`, `require`, `in-ns`, `alias`, `load-file`, `with-out-str`, `await` |
| Versioned lookup | resolution of `sym@commit` |

Violations surface as `EvalError::ForbiddenEffect(operation)`, naming what was
attempted. A host can install the guard directly around its own evaluation if it
wants that denylist without the rest of the `cljrs-tx` profile — but note that
the guard alone is one layer, not the whole boundary; the fresh environment and
the arena are the other two.

Where guest code genuinely needs to read host state, `execute_with_host` interns
each `HostApi` entry as a namespaced native function (`my.host/lookup`). That is
the *only* seam through the boundary: arguments are serialized out and validated
as pure data, results are validated and cloned back in, and no live handle ever
enters the arena. Keep those functions deterministic, side-effect-free reads.

Build `cljrs-tx` with its `no-gc` feature (`cargo test -p cljrs-tx --features
no-gc`); it is off by default so a normal workspace build does not unify every
dependency into arena mode.

### What the transaction profile still does not give you

- **Not a hard RSS limit.** `memory_bytes` covers arena object boxes and
  out-of-line memory reported by `Trace::gc_size_extra`. Ordinary Rust
  allocations outside traced values are not fully charged. An adversarial
  address-space ceiling still needs a process or WASM sandbox around it.
- **Not a wall-clock timeout.** Gas bounds work, not time.
- **Not a stack guarantee by itself.** `call_depth` caps interpreted frames; the
  thread's stack still has to be big enough for the depth you allow.
- **No I/O, no async, no JIT, no Rust interop** inside it, by construction — that
  is the point, but it means transaction functions are pure computations over
  data, not general programs.

## A checklist

| Guest code is… | Do this |
|---|---|
| Yours, in your repo | `Tiered` + JIT, GC limits from the environment. Nothing else. |
| Yours, but a runaway loop would be embarrassing | Add a gas meter around each top-level evaluation. |
| Written by users who can already run code on the box | Gas meter, explicit `GcConfig`, empty source paths, install only the extensions you need. |
| Genuinely untrusted | `cljrs-tx` with `TxLimits`, host reads through `HostApi`, and an OS-level limit around the process. |
