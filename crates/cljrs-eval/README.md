# cljrs-eval

**Deprecated re-export shim.** This package holds no code.

IR-accelerated (tiered) evaluation — the tier-1 IR interpreter, IR cache, lowering worker, and JIT dispatch state — moved into `cljrs_runtime::tiered` in Stage 2 of
[`docs/crate-consolidation-plan.md`](../../docs/crate-consolidation-plan.md).
This package exists only so downstream packages can migrate one at a time;
Stage 6 removes it.

Replace `cljrs_eval::x` with `cljrs_runtime::tiered::x`, and the
`cljrs-eval` dependency with `cljrs-runtime`.

---

## File layout

```
src/
  lib.rs — `pub use cljrs_runtime::tiered::*;`
```

---

## Public API

Everything public in [`cljrs_runtime::tiered`](../cljrs-runtime/README.md), re-exported
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
