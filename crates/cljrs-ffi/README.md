# cljrs-ffi

## Purpose

Registers `clojure.rust.ffi`: open a C ABI shared library and call its symbols
from Clojure, without libffi and without building the library against cljrs.

## Status

Interop extension. Implemented on x86_64 and aarch64 Linux and macOS; on every
other target (wasm, Windows) `init` registers nothing. Enabled in the CLI by
the default `ffi` feature, in both the interpreter and AOT (`cljrs compile`).

User documentation, the type table, error data and the calling-convention
rationale: [`docs/book/src/rust-interop/c-ffi.md`](../../docs/book/src/rust-interop/c-ffi.md).

## File layout

| Path | Description |
|---|---|
| `src/lib.rs` | `NS`, `SUPPORTED`, `init`; gates `imp` to supported targets |
| `src/imp/mod.rs` | The seven builtins and their registration |
| `src/imp/library.rs` | `LibInner` (the shared `libloading::Library` slot), `LibHandle` (the `NativeObject` value) |
| `src/imp/sig.rs` | `CType`, `Signature::resolve` (checked once), `Signature::pack` into the two register files |
| `src/imp/invoke.rs` | The two call shapes; C string and byte copies |
| `src/imp/error.rs` | The `{:ffi/error ...}` `ex-info` constructors |
| `tests/fixture/ffi_fixture.c` | C fixture, compiled by the tests with `$CC` (default `cc`) |
| `tests/common/mod.rs` | Builds the fixture; an environment with stdlib, `clojure.data.json`, `clojure.rust.ffi` |
| `tests/ffi.rs` | Transaction-policy denial, use after `close`, registration |
| `tests/clojure_tests.rs` | Runs the clojure.test namespaces under `test/` |
| `test/clojure/rust/ffi_test.cljrs` | Fixture calls, every error path, live hive C ABI check (skipped when absent) |

## Public API

```rust
pub const NS: &str = "clojure.rust.ffi";
pub const SUPPORTED: bool;                 // this target has the namespace
pub fn init(globals: &Arc<GlobalEnv>);     // idempotent; no-op when !SUPPORTED
```

Clojure surface: `open`, `close`, `sym`, `function`, `call`, `string`, `bytes`.
