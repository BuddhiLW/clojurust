# `tests/cljrs` — the cljrs-owned clojure.test tree

## Purpose

The language-surface tests, written in the language: what `deftype`, `defmulti`,
protocol impl positions and reader metadata mean.

## Status

Phase 4+ (evaluator and special forms). Implemented and wired into CI.

## The split

| Belongs here | Belongs in Rust |
|---|---|
| Macro expansion, dispatch, arity, namespace resolution, metadata | `EvalError` / `ValueError` variants and their payloads |
| Anything an assertion written in Clojure can see | Primitive builtin contracts, GC behaviour, tier and lowering behaviour |

`clojure-test-suite/` is a vendored mirror of the upstream `clojure/core-test`
suite. Our own tests do not go in it.

## `.cljc`, not `.cljrs`

Both extensions are discovered by `cljrs test` and `cljrs compile --test`.
`.cljc` also loads on JVM Clojure, so the same corpus runs on both and the JVM
answers as the oracle. A test passing on one side only is a divergence, and is
written as a reader conditional with both branches asserted. The platform key
for this runtime is `:rust`.

Recorded divergences:

- `deftype` mutable fields: cljrs allows `(set! (.-field inst) v)` from outside
  the type's own methods; the JVM raises `IllegalArgumentException`.
- `(defmulti m)` with no dispatch function: cljrs refuses the form; the JVM
  accepts it and fails at the call.

## File layout

| Path | Description |
|---|---|
| `cljrs/lang_test/deftype.cljc` | Positional constructor, field access, protocol method bodies, mutable fields and `set!` |
| `cljrs/lang_test/record_predicate.cljc` | `record?` over defrecord, deftype, reify, maps and `assoc` |
| `cljrs/lang_test/protocol_impl.cljc` | A qualified protocol in an impl position, across the five impl sites |
| `cljrs/lang_test/protocol_head.cljc` | `defprotocol` docstrings and `:arglists`; grouped multi-arity bodies in `extend-type` / `extend-protocol` |
| `cljrs/lang_test/queue.cljc` | `PersistentQueue` seqability (`seq`, `vec`, `into`, `map`, `reduce`, `first`/`rest`, `nth`) and `clojure.lang.PersistentQueue/EMPTY` |
| `cljrs/lang_test/multimethod.cljc` | Hierarchy dispatch, specificity, `prefer-method`, the method table, the `defmulti` head |
| `cljrs/lang_test/meta_transparency.cljc` | Property oracle: `^meta F` evaluates to what `F` evaluates to |
| `cljrs/lang_test/fixture/proto.cljc` | A protocol in a namespace of its own; no tests |
| `../cljrs-jvm-oracle.clj` | Runs the corpus on JVM Clojure, deriving namespaces from the tree |

## Running it

```sh
cargo run -p cljrs test --src-path ./tests/cljrs
cargo run -p cljrs compile -o cljrs-tests --test ./tests/cljrs && ./cljrs-tests
clojure -Sdeps '{:paths ["tests/cljrs"]}' -M tests/cljrs-jvm-oracle.clj
cargo test -p cljrs --test self_hosted
```

`crates/cljrs/tests/self_hosted.rs` is the cargo gate over the interpreter leg.
The AOT leg is not run from cargo, because it invokes cargo itself to build a
harness crate; CI runs it separately.

## Adding a test

Assert what Clojure means — the JVM leg is the oracle. Where cljrs deliberately
differs, use a reader conditional and assert both branches.
