# Calling C libraries (`clojure.rust.ffi`)

`clojure.rust.ffi` opens any C ABI shared library (C, C++ `extern "C"`, Go
`c-shared`, Zig, Rust `cdylib`) and calls its symbols from Clojure. Unlike the
[`cljrs_init` plugin ABI](registry.md), the library does not have to be built
against cljrs's crate graph.

The namespace ships in the `cljrs-ffi` crate, enabled by the CLI's default `ffi`
feature in both `cljrs run` and `cljrs compile`. It exists on x86_64 and aarch64
Linux and macOS only; on wasm and Windows it is absent.

```clojure
(require '[clojure.rust.ffi :as ffi])

(def libm (ffi/open "libm.so.6"))
(def cos (ffi/function libm "cos" [:double] :double))
(cos 0.0)                                 ;=> 1.0
(map cos [0.0 1.0])                      ; a bound function is an ordinary fn
(ffi/call libm "pow" [:double :double] :double 2.0 10.0) ;=> 1024.0
(ffi/close libm)
```

The same API, names and error data exist on cljw as `cljw.ffi`, so a portable
adapter needs one reader conditional:

```clojure
(:require #?(:cljw [cljw.ffi :as ffi] :rust [clojure.rust.ffi :as ffi]))
```

## API

| Function | Result |
|---|---|
| `(open path)` | A library handle. `path` is absolute, relative, or a bare soname (the system search path). |
| `(close lib)` | `nil`. Idempotent. Every later use of `lib` or of a function built from it throws `{:ffi/error :closed}`. |
| `(sym lib name)` | The symbol's address, as an integer. |
| `(function lib name arg-types ret-type)` | An ordinary Clojure fn calling the C symbol. Types and the symbol are resolved once, here. |
| `(call lib name arg-types ret-type & args)` | `((function lib name arg-types ret-type) args...)`. |
| `(string ptr)` | A copy of the NUL-terminated UTF-8 string at `ptr`; `nil` for 0 or `nil`. |
| `(bytes ptr n)` | A byte array holding a copy of the `n` bytes at `ptr`. |

## Types

| Type | As an argument | As a return |
|---|---|---|
| `:void` | refused | `nil` |
| `:int` | C `int`: an integer, truncated to 32 bits | sign-extended from the low 32 bits |
| `:long` | `int64_t` | integer |
| `:double` | `double`; an integer is converted | double |
| `:pointer` | an address integer; `nil` passes NULL | the address; NULL is `0` |
| `:string` | a NUL-terminated UTF-8 copy, valid during the call; `nil` passes NULL | a copy of the C string; NULL is `nil`. Never freed. |
| `:bytes` | a pointer to a copy of a byte array, valid during the call | refused |

When the callee allocates the string it returns, declare the return `:pointer`,
copy it with `string`, then call the library's own free function on the pointer.

## Errors

Every failure is an `ex-info` whose data carries `:ffi/error`:

| `:ffi/error` | When | Other keys |
|---|---|---|
| `:open` | the library does not load | `:path`, `:reason` (the loader's message) |
| `:symbol` | the symbol is not exported | `:symbol` |
| `:signature` | at `function`: unknown type, `:void` argument, `:bytes` return, too many arguments | `:reason`, sometimes `:type` |
| `:arity` | a call with the wrong argument count | `:expected`, `:got` |
| `:arg-type` | an argument of the wrong kind | `:index`, `:type` |
| `:closed` | any use after `close` | |

All of `clojure.rust.ffi` is denied inside a transaction function: loading and
calling native code is an effect no retry can undo.

## Example: the hive C ABI

A library exporting `char* hive_call(const char* op, const char* json)` and
`void hive_free(char*)` returns a JSON envelope the caller must free:

```clojure
(require '[clojure.rust.ffi :as ffi]
         '[clojure.data.json :as json])

(def lib (ffi/open (str (System/getenv "HIVE_POLYGLOT_NATIVE") "/libvectorcraft.so")))

(defn hive-call [op arg]
  (let [p (ffi/call lib "hive_call" [:string :string] :pointer op (json/write-str arg))]
    (try (json/read-str (ffi/string p))
         (finally (ffi/call lib "hive_free" [:pointer] :void p)))))

(hive-call "ops" {})  ;=> {"ok" true, "value" ...}
```

The runnable version is `examples/ffi/hive_call.cljrs`.

## How a call is made: two call shapes, no libffi

On x86_64 SysV and aarch64 AAPCS64, integer-class and floating-point arguments
are assigned to two separate register files, each in order of appearance within
its class. A callee reads only the registers its own prototype names, and extra
argument registers are ignored. So every signature with at most 6 integer-class
arguments (`:int :long :pointer :string :bytes`) and at most 8 `:double`
arguments can be called through one of two function-pointer types:

```rust
extern "C" fn(usize, usize, usize, usize, usize, usize,
              f64, f64, f64, f64, f64, f64, f64, f64) -> usize // all non-double returns
extern "C" fn(usize, usize, usize, usize, usize, usize,
              f64, f64, f64, f64, f64, f64, f64, f64) -> f64   // :double return
```

`function` packs integer-class arguments in order into the six `usize` slots and
doubles into the eight `f64` slots, zeroing the rest. This is why the limits are
what they are: 6 and 8 are the argument registers of the smaller ABI (SysV), and
anything that would spill to the stack, be passed by varargs rules, or need a
float32 or struct-by-value classification cannot be expressed. Those signatures
are refused at `function` time with `{:ffi/error :signature}` rather than called
wrongly. Windows x64 shares one register sequence between both classes, so the
trick does not hold there and the namespace is absent.
