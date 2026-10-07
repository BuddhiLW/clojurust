# Project Setup

A mixed Rust/Clojure project needs three things: a `cljrs.edn` that points to
the Rust crate, a `Cargo.toml` with the right crate type, and an init
entry point.

## Directory layout

```
my-project/
├── cljrs.edn          # Clojure project config (source paths, :rust key)
├── Cargo.toml         # Rust crate manifest
├── src/
│   ├── lib.rs         # Rust source — defines the init fn and native fns
│   └── main.cljrs     # Clojure entry point
```

The Rust crate and the `cljrs.edn` file can live in the same directory (`:crate
"."`) or in a subdirectory (`:crate "native"`).

## `cljrs.edn`

Add a `:rust` map to the top-level config:

```clojure
{:paths ["src"]

 :rust {:crate "."                       ; path to Cargo.toml directory
        :init  "my_project::cljrs_init_my_project"} ; Rust path to the init function
}
```

| Key | Required | Description |
|---|---|---|
| `:crate` | yes | Path to the directory containing the user's `Cargo.toml`. Relative to `cljrs.edn`. |
| `:init` | yes | Fully-qualified Rust path to the init function, e.g. `"my_crate::cljrs_init_my_crate"`. The first `::` segment is used as the crate name. |

## `Cargo.toml`

The user crate must be a library with `cdylib` output (for interpreter-mode
dynamic loading) and, optionally, `rlib` output (for AOT static linking):

```toml
[package]
name    = "my_project"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
cljrs-interop = { path = "/path/to/cljrs/crates/cljrs-interop" }
```

> **Note:** `cdylib` produces the `.so`/`.dylib`/`.dll` loaded by `cljrs run`.
> `rlib` allows `cljrs compile` to link the crate statically into the AOT
> binary. Both can coexist in `crate-type`.

## The init entry point

The init function receives a `*mut Registry` pointer and registers all native
functions. It must have C linkage so the dynamic linker can find it by name:

```rust
use cljrs_interop::{Registry, wrap_fn1, wrap_fn2};

#[no_mangle]
pub extern "C" fn cljrs_init_my_project(registry: *mut Registry) {
    let r = unsafe { &mut *registry };

    r.define("my.project/greet",
        wrap_fn1("greet", |name: String| {
            Ok::<String, String>(format!("Hello, {name}!"))
        }));

    r.define("my.project/add",
        wrap_fn2("add", |a: i64, b: i64| Ok::<i64, String>(a + b)));
}
```

The function name in `:rust :init` (`"my_project::cljrs_init_my_project"`) must
match the Rust function name used in `#[no_mangle]`
(`cljrs_init_my_project`). The crate prefix (`my_project`) is the Rust
*identifier* for your crate — the `[package] name` with hyphens replaced by
underscores, which is how Rust source always spells it.

You do **not** have to rename your package to match. A hyphenated package name
(`my-plugin`) is the Rust norm and works: the AOT harness reads the real name
from your `Cargo.toml` and emits `package = "my-plugin"` alongside the
identifier, so Cargo resolves the dependency while generated source keeps
calling the crate `my_plugin`.

### Name the symbol after your crate

The convention is `cljrs_init_<crate>`, and the reason is `#[no_mangle]`. An
unmangled symbol is global to the whole process image, so two extension crates
that both call theirs `cljrs_init` export the *same* symbol.

When `cljrs compile` links two such extensions into one AOT binary, the result
depends on archive extraction. If another reference pulls both defining object
files into the link, the build fails with a duplicate-symbol error. Otherwise,
the linker can select one definition and route both Rust paths to it, silently
skipping the other crate's registration. Codegen-unit partitioning and link
order can change which outcome occurs.

Nothing in cljrs hardcodes the name. The loader takes the last `::` segment of
`:rust :init` and looks that up, so the symbol name is data: any spelling works
as long as it is unique across every extension that might share a binary.
Deriving it from the crate name is the cheapest way to guarantee that.

## Calling native functions from Clojure

Native functions registered under `"my.project/greet"` are visible in Clojure
as `my.project/greet`. No `require` is needed unless you want a namespace alias:

```clojure
; Direct qualified call
(my.project/greet "world")       ; => "Hello, world!"

; With a require alias
(ns my.app
  (:require [my.project :as native]))

(native/add 3 4)                 ; => 7
```

The namespace `my.project` is created automatically when the init function runs;
you do not need to create or load a Clojure file for it.

## ABI-checked project crates

For new crates, use `cljrs_interop::export_init!` instead of the manual
`#[no_mangle]` init above. The macro exports both the uniquely named init
symbol and `cljrs_dylib_abi`, with the same cljrs-version, rustc-version,
and debug/release profile fingerprint checked by the pinned-package loader:

```rust
cljrs_interop::export_init!(cljrs_init_my_project, |r: &mut cljrs_interop::Registry| {
    cljrs_interop::register_exports(r);
});
```

Keep `:rust :init` set to `"my_project::cljrs_init_my_project"`. Include
`cljrs-interop` as a dependency in `Cargo.toml`; its build script captures
`rustc -V` when compiling the crate. Only call the macro once per cdylib.
`cljrs build-native` prints the artifact's fingerprint to stderr (or reports
that the symbol is absent) and prints the library path to stdout.

If the library's fingerprint differs from the running CLI, startup stops
before calling init, with both fingerprints and `rebuild with \`cljrs
build-native\`` in the error. Rebuild using the *same* cljrs version, rustc,
and profile as the binary. Old libraries without `cljrs_dylib_abi` still load
but warn once per load that their unchecked Rust ABI may crash. Set
`CLJRS_NATIVE_STRICT=1` to refuse them instead; rebuild with the macro to
safely enable strict mode. A fingerprint is not a complete crate-graph check:
keep your dependency lockfile aligned with the binary, especially `archery`
and `rpds`.

There is no `cljrs new` command or generated Cargo manifest today; create a
crate as above and pin `archery = "=1.2.2"` and `rpds = "=1.2.1"` if you use
them directly, matching the workspace lockfile.
