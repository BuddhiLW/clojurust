# cljrs-builtins

**Deprecated re-export shim.** This package holds no code.

The native `clojure.core` builtins and the embedded Clojure bootstrap source moved into `cljrs_runtime::builtins` in Stage 2 of
[`docs/crate-consolidation-plan.md`](../../docs/crate-consolidation-plan.md).
This package exists only so downstream packages can migrate one at a time;
Stage 6 removes it.

Replace `cljrs_builtins::x` with `cljrs_runtime::builtins::x`, and the
`cljrs-builtins` dependency with `cljrs-runtime`.

---

## File layout

```
src/
  lib.rs — `pub use cljrs_runtime::builtins::*;`
```

---

## Public API

Everything public in [`cljrs_runtime::builtins`](../cljrs-runtime/README.md), re-exported
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
