# cljrs-interp

**Deprecated re-export shim.** This package holds no code.

The tree-walking interpreter — special forms, macro expansion, destructuring, and the recur trampoline — moved into `cljrs_runtime::interp` in Stage 2 of
[`docs/crate-consolidation-plan.md`](../../docs/crate-consolidation-plan.md).
This package exists only so downstream packages can migrate one at a time;
Stage 6 removes it.

Replace `cljrs_interp::x` with `cljrs_runtime::interp::x`, and the
`cljrs-interp` dependency with `cljrs-runtime`.

---

## File layout

```
src/
  lib.rs — `pub use cljrs_runtime::interp::*;`
```

---

## Public API

Everything public in [`cljrs_runtime::interp`](../cljrs-runtime/README.md), re-exported
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
