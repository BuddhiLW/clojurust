# `tests/cljrs` — the cljrs-owned clojure.test tree

## Purpose

The **language** half of this project's tests, written in the language.

## Status

Phase 4+ (evaluator and special forms). Implemented and wired into CI.

## Why this tree exists

Every language-surface test used to be a Rust string literal run through an
`eval_pr` helper that asserted on the **printed form** of a value:

```rust
assert_eq!(eval_pr("(defmulti area :shape) …"), "9");
```

The subject is Clojure, the frame is Rust, and the assertion is on a rendering
rather than on a value. That cost real correctness: an emptied method table
prints `nil`, not `()`, and the wrong expectation went in on the first pass. The
check that says what is meant is `(is (= 0 (count (methods area))))`, and it is
only writable from inside the language.

## The split

| Belongs here | Belongs in Rust |
|---|---|
| Macro expansion, dispatch, arity, namespace resolution, metadata behaviour | `EvalError` / `ValueError` variants and their payloads |
| Anything an assertion written in Clojure can see | Primitive builtin contracts, GC behaviour, tier/lowering behaviour |

`clojure-test-suite/` is **not** this tree: it is a vendored mirror of the
upstream `clojure/core-test` suite, so our own tests do not belong in it.

## Why `.cljc` and not `.cljrs`

Both extensions are discovered by `cljrs test` and `cljrs compile --test`.
`.cljc` is chosen so the **same corpus also runs on JVM Clojure**, which makes
this tree a differential oracle rather than only a regression suite:

- passing on both sides pins agreed behaviour;
- passing on one side is a divergence, and has to be argued for in a reader
  conditional instead of being discovered years later.

The platform key for this runtime is `:rust`.

Two divergences are recorded so far, both in both directions:

- `deftype` mutable fields — cljrs allows `(set! (.-field inst) v)` from
  outside the type's own methods; the JVM raises `IllegalArgumentException`.
- `(defmulti m)` with no dispatch function — cljrs refuses the form; the JVM
  accepts it and fails only at the call.

## File layout

| Path | Description |
|---|---|
| `cljrs/lang_test/deftype.cljc` | `deftype`: positional constructor, field access, protocol method bodies, `^:unsynchronized-mutable` / `^:volatile-mutable` fields and `set!` |
| `cljrs/lang_test/protocol_impl.cljc` | A protocol named in an impl position resolves through its own namespace — the five impl sites, aliased and fully qualified |
| `cljrs/lang_test/multimethod.cljc` | `defmulti` / `defmethod`: hierarchy dispatch, specificity, `prefer-method`, the method table as data, and the `defmulti` head (docstring, attr-map, precedence) |
| `cljrs/lang_test/meta_transparency.cljc` | Property oracle: `^meta F` evaluates to what `F` evaluates to, at every annotation site |
| `cljrs/lang_test/fixture/proto.cljc` | A protocol in a namespace of its own; definitions only, no tests |
| `../cljrs-jvm-oracle.clj` | Runs this corpus on JVM Clojure; derives namespaces from the tree so it cannot drift from `cljrs`'s own discovery |

## Running it

```sh
# interpreter
cargo run -p cljrs test --src-path ./tests/cljrs

# AOT-compiled harness
cargo run -p cljrs compile -o cljrs-tests --test ./tests/cljrs && ./cljrs-tests

# the JVM oracle
clojure -Sdeps '{:paths ["tests/cljrs"]}' -M tests/cljrs-jvm-oracle.clj

# the cargo gate over the interpreter leg
cargo test -p cljrs --test self_hosted
```

A plain `cargo test` covers the interpreter leg through
`crates/cljrs/tests/self_hosted.rs`, so a contributor who never runs the binary
still sees a language-level break. The AOT leg is not run from cargo, because
it invokes `cargo` itself to build a harness crate; CI runs it separately.

## Adding a test

Put it here if a reader could ask "what does Clojure do?" and the answer is the
point. Write the assertion against what **Clojure** means — the JVM leg is the
oracle — and if cljrs deliberately differs, say so in a reader conditional with
both branches asserted, so the day the divergence closes the `:rust` branch
fails and names itself.
