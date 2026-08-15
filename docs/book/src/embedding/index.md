# Embedding clojurust

Everything the `cljrs` CLI does — `run`, `repl`, `eval`, `test`, `nrepl` — it
does through a public Rust API that your own program can call. Embedding means
building a clojurust runtime inside a larger Rust application and evaluating
Clojure in it: a scripting layer for a game or an editor, a rules engine whose
policies ship separately from the binary, a plugin host, or a service that
exposes an nREPL port so operators can inspect it live.

The API is small. Four steps cover every embedding:

1. **Build** a runtime with [`Runtime::builder()`](runtime-builder.md) — this is
   where you pick the [execution mode](execution-modes.md), source paths, and GC
   limits.
2. **Attach** the tiers and [extensions](extensions.md) you want: the JIT, the
   standard library, async/IO/networking, your own native functions.
3. **Evaluate** Clojure forms in an `Env` and [move values across the
   boundary](evaluating.md).
4. **Bound** what the guest code may do, if it is not fully trusted — see
   [Limits & sandboxing](sandboxing.md).

## A minimal host

```rust
use cljrs_reader::Parser;
use cljrs_runtime::tiered::{Env, EvalError, eval};
use cljrs_runtime::{ExecutionMode, Runtime};
use cljrs_value::Value;

fn main() {
    // 1. Build.
    let runtime = Runtime::builder()
        .execution_mode(ExecutionMode::Tiered)
        .source_paths(vec!["src".into()])
        .build()
        .expect("bootstrap clojure.core");

    // 2. Attach the standard library.
    cljrs_stdlib::install(&runtime);

    // 3. Evaluate.
    let mut env = runtime.env("user");
    let value = eval_str(&mut env, "(+ 1 2)").expect("eval");
    println!("{value}");   // => 3
}

/// Parse `src` and evaluate every form in it, returning the last value.
fn eval_str(env: &mut Env, src: &str) -> Result<Value, EvalError> {
    let mut parser = Parser::new(src.to_string(), "<host>".to_string());
    let forms = parser.parse_all().map_err(EvalError::Read)?;
    let mut result = Value::Nil;
    for form in &forms {
        // One allocation frame per top-level form: it roots everything the
        // form allocates, and releases it when the form is done.
        let _frame = cljrs_gc::push_alloc_frame();
        result = eval(form, env)?;
    }
    Ok(result)
}
```

That is a complete embedding. `build()` registers native `clojure.core`,
evaluates the Clojure bootstrap, creates a `user` namespace that refers it, and
raises the runtime to its target tier; `install` adds `clojure.string`,
`clojure.set`, `clojure.test`, and the rest.

## Which crates you depend on

| Crate | You need it for |
|---|---|
| `cljrs-runtime` | `Runtime`, `RuntimeBuilder`, `Env`, `eval` — always |
| `cljrs-value` | the `Value` enum you pass in and get back — always |
| `cljrs-reader` | `Parser`, turning source text into forms — always |
| `cljrs-gc` | `GcConfig`, `push_alloc_frame` — almost always |
| `cljrs-stdlib` | `clojure.string`, `clojure.set`, `clojure.edn`, `clojure.test`, … |
| `cljrs-compiler` | the JIT tier (`jit::install`), and AOT compilation |
| `cljrs-async`, `cljrs-io`, `cljrs-net`, `cljrs-charset`, `cljrs-base64` | `core.async`, file I/O, sockets, codecs |
| `cljrs-interop` | exposing your Rust functions to Clojure |
| `cljrs-nrepl` | letting editors connect to your embedded runtime (`cljrs-lsp` is syntactic only and needs no runtime) |
| `cljrs-tx` | running untrusted pure functions under hard limits |

None of these pull in `clap`, `rustyline`, or the rest of the CLI — the `cljrs`
binary crate is one consumer of this API, not a layer under it.

## Two rules that shape every embedding

**A runtime is confined to one thread.** `GcPtr` — and therefore `Value`, `Env`,
`GlobalEnv`, and `Runtime` — is `!Send`. This is enforced by the compiler, not by
convention: `fn assert_send<T: Send>()` fails to compile for `Runtime`. Build
the runtime on the thread that will evaluate in it, and keep it there. Each
thread that hosts a runtime gets its own GC heap and collects independently, so
several runtimes on several threads scale without coordinating; values move
between them by copying, never by aliasing. See [Worker
isolation](../async-io/isolation.md) for that boundary, and
[Evaluating](evaluating.md#threads-and-runtimes) for the host-side rules.

**Values are garbage-collected, and the collector needs to see yours.** A
`Value` sitting in a host struct is not a root by itself. Anything reachable
from a namespace is safe; anything else needs an allocation frame or an explicit
root guard for as long as you hold it. [Evaluating](evaluating.md#gc-discipline)
spells out the three cases.

## Chapter overview

| Page | Contents |
|---|---|
| [The runtime builder](runtime-builder.md) | Every builder option, what `build()` does, the `Runtime` handle |
| [Execution modes](execution-modes.md) | `TreeWalk`, `Tiered`, `TieredNoJit`, `NoGcTransaction`; tier promotion; JIT thresholds |
| [Extensions & native code](extensions.md) | Installing the stdlib, async/IO/net, and your own Rust functions |
| [Evaluating code](evaluating.md) | Parsing, `Env`, calling Clojure from Rust, marshalling, errors, GC discipline |
| [Limits & sandboxing](sandboxing.md) | Memory limits, gas metering, depth caps, and what is *not* sandboxed |
