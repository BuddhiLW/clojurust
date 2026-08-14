# cljrs-env

**Deprecated re-export shim.** This package holds no code.

The namespace and environment layer — `Env`, `GlobalEnv`, dynamic bindings, GC roots, the namespace loader, gas metering, and the transaction policy — moved into `cljrs_runtime::env` in Stage 2 of
[`docs/crate-consolidation-plan.md`](../../docs/crate-consolidation-plan.md).
This package exists only so downstream packages can migrate one at a time;
Stage 6 removes it.

Replace `cljrs_env::x` with `cljrs_runtime::env::x`, and the
`cljrs-env` dependency with `cljrs-runtime`.

---

## File layout

```
src/
  lib.rs — `pub use cljrs_runtime::env::*;`
```

---

## Public API

Everything public in [`cljrs_runtime::env`](../cljrs-runtime/README.md), re-exported
unchanged. See that crate's README for the documented surface.

---

## Features

| Feature | Effect |
|---|---|
| `no-gc` | Forwards to `cljrs-runtime/no-gc`. |

---

## Dependencies

| Crate | Role |
|-------|------|
| `cljrs-runtime` (workspace) | the package this shim re-exports |
