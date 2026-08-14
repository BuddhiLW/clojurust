# cljrs-runtime

Placeholder package. It contains no code and nothing in the workspace depends
on it.

**Status:** empty stub, reserved. Stage 2 of
[`docs/crate-consolidation-plan.md`](../../docs/crate-consolidation-plan.md)
replaces it with the merged runtime — `cljrs-env`, `cljrs-builtins`,
`cljrs-interp`, and `cljrs-eval` folded into `env`, `builtins`, `interp`, and
`tiered` modules behind one `Runtime` builder.

The work this README used to describe — registering `clojure.core` functions
and macros, and the concurrency primitives — was implemented elsewhere and is
live today:

| What | Where it actually lives |
|---|---|
| Core functions, type predicates, collection and seq operations | [`cljrs-builtins`](../cljrs-builtins) |
| Core macros, special forms, destructuring | [`cljrs-interp`](../cljrs-interp) |
| `atom` / `ref` / `agent` / `future` / `promise` | [`cljrs-builtins`](../cljrs-builtins) |
| Namespace registry, dynamic bindings, GC roots | [`cljrs-env`](../cljrs-env) |
| Embedded `clojure.*` namespaces | [`cljrs-stdlib`](../cljrs-stdlib) |

---

## File layout

```
src/
  lib.rs    — doc-comment stub; no items
```

---

## Public API

None. The crate exports nothing.

---

## Features

| Feature | Effect |
|---|---|
| `no-gc` | Forwards to `cljrs-gc/no-gc`. Inert while the crate is empty. |

---

## Dependencies

| Crate | Role |
|-------|------|
| `cljrs-types` (workspace) | unused by the stub; kept for the Stage 2 merge |
| `cljrs-gc` (workspace) | unused by the stub; carries the `no-gc` feature forward |
| `cljrs-eval` (workspace) | unused by the stub; kept for the Stage 2 merge |
