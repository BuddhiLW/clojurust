# cljrs-raster

2D vector rasterization for Clojurust, wrapping [`tiny-skia`](https://crates.io/crates/tiny-skia)
(a pure-Rust port of a Skia subset) and registering the `cljrs.raster` namespace.

## Purpose

Give Clojure code an anti-aliased software rasterizer: paths, fills, strokes,
gradients, blend modes, affine transforms, PNG encode/decode, and raw RGBA in
and out. The raw-RGBA path is what lets `cljrs-ffmpeg` turn a sequence of drawings into
a video without an intermediate file per frame: `(ff/write-frame! w (r/rgba-bytes c))`.

## Status

Phase 9 (Rust Interop) — implemented. An **extension**: statically linked into
the `cljrs` binary via its Cargo feature (enabled by default) and into
AOT-compiled binaries through `cljrs::extensions::default_set`.

It is not a dynamically loadable plugin, and that is deliberate. A plugin
exports an unmangled `cljrs_init`, and a link may define that symbol exactly
once; since Cargo unifies features across a whole build, no feature gate keeps
two such crates apart under `cargo build --workspace`. `cljrs-base64` already
holds that slot among the CLI's dependencies. Extensions register through
`init`/`register` and compose without that constraint.

## File layout

| File | Description |
|---|---|
| `src/lib.rs` | `NS`, `init`, `register`, `cljrs_init`; every `registry.define` call |
| `src/args.rs` | `Value` → Rust coercions: numbers, options maps, coordinate vectors |
| `src/color.rs` | Colour specs — named keywords, `#hex` strings, integer and float channel vectors |
| `src/paint.rs` | Options maps → `Paint`, `Stroke`, `FillRule`, `Transform`; gradients and blend modes |
| `src/path.rs` | The `PathBuilder` and `Path` native objects, plus one-shot path constructors |
| `src/canvas.rs` | The `Canvas` native object: drawing, pixels, PNG and raw-RGBA codecs |
| `test/cljrs/raster_test.cljrs` | `clojure.test` suite (35 tests, 65 assertions) |
| `tests/clojure_tests.rs` | Rust harness that drives the Clojure suite |

## Public API

### Rust

```rust
/// Clojure namespace registered by this crate.
pub const NS: &str = "cljrs.raster";

/// Register `cljrs.raster` into `globals`. Idempotent.
pub fn init(globals: &Arc<GlobalEnv>);

/// Register every function through an existing `Registry`.
pub fn register(registry: &mut Registry);

/// Straight-RGBA bytes for a value, if it is a canvas. For a Rust host that
/// wants the frame bytes without going through the Clojure var.
pub fn canvas_rgba(v: &Value) -> Option<Result<Vec<u8>, String>>;

/// A canvas's dimensions, for callers outside this crate.
pub fn canvas_size(v: &Value) -> Result<(u32, u32), String>;

pub fn as_canvas(v: &Value) -> Result<&Canvas, String>;

/// Opaque native objects. Type tags: `"Canvas"`, `"Path"`, `"PathBuilder"`.
pub struct Canvas;
pub struct RasterPath;
pub struct RasterPathBuilder;
```

### Clojure namespace: `cljrs.raster`

#### Canvas

| Symbol | Signature | Returns | Description |
|---|---|---|---|
| `canvas` | `(canvas w h)` | `Canvas` | A transparent `w`×`h` surface |
| `canvas?` | `(canvas? v)` | `Boolean` | |
| `color` | `(color spec)` | `[r g b a]` | Normalize any colour spelling to 0–255 integers |
| `width` / `height` | `(width c)` | `Long` | |
| `clone-canvas` | `(clone-canvas c)` | `Canvas` | An independent copy |
| `clear!` | `(clear! c colour)` | `c` | Fill every pixel |
| `pixel` | `(pixel c x y)` | `[r g b a]` or `nil` | Straight (un-premultiplied) channels, `nil` outside the canvas |
| `set-pixel!` | `(set-pixel! c x y colour)` | `c` | Throws outside the canvas |

#### Drawing

Every drawing function takes an optional trailing options map and returns the
canvas, so calls thread through `->` and `doto`.

| Symbol | Signature |
|---|---|
| `fill-rect!` | `(fill-rect! c x y w h)` `(fill-rect! c x y w h opts)` |
| `stroke-rect!` | `(stroke-rect! c x y w h [opts])` |
| `fill-circle!` / `stroke-circle!` | `(fill-circle! c cx cy r [opts])` |
| `stroke-line!` | `(stroke-line! c x1 y1 x2 y2 [opts])` |
| `fill-path!` / `stroke-path!` | `(fill-path! c path [opts])` |
| `draw-canvas!` | `(draw-canvas! dst src x y [opts])` — composite one canvas onto another |

#### Paths

| Symbol | Signature | Returns |
|---|---|---|
| `path-builder` | `(path-builder)` | `PathBuilder` |
| `move-to!` / `line-to!` | `(move-to! b x y)` | the builder |
| `quad-to!` | `(quad-to! b x1 y1 x y)` | the builder |
| `cubic-to!` | `(cubic-to! b x1 y1 x2 y2 x y)` | the builder |
| `close-path!` | `(close-path! b)` | the builder |
| `push-rect!` / `push-oval!` | `(push-rect! b x y w h)` | the builder |
| `push-circle!` | `(push-circle! b cx cy r)` | the builder |
| `finish-path` | `(finish-path b)` | `Path` — **consumes** the builder |
| `path?` | `(path? v)` | `Boolean` |
| `rect` / `oval` | `(rect x y w h)` | `Path` |
| `circle` | `(circle cx cy r)` | `Path` |
| `polygon` / `polyline` | `(polygon [[x y] …])` | `Path` (closed / open) |

A builder is single-use: `finish-path` takes its contents, and every later call
on that builder throws rather than silently drawing nothing.

#### Encoding

| Symbol | Signature | Returns |
|---|---|---|
| `encode-png` | `(encode-png c)` | `ByteArray` |
| `decode-png` | `(decode-png bytes)` | `Canvas` |
| `save-png!` | `(save-png! c path)` | the path |
| `load-png` | `(load-png path)` | `Canvas` |
| `rgba-bytes` | `(rgba-bytes c)` | `ByteArray` — straight RGBA, 4 bytes/pixel, row-major |
| `from-rgba` | `(from-rgba w h bytes)` | `Canvas` |

Bytes are returned as `ByteArray` rather than `ByteBlob`: core's `alength`,
`aget` and `vec` reach into the former and not the latter, and this matches what
`cljrs.base64/decode` returns.

`bytes` arguments accept a `ByteArray`, a `ByteBlob`, or a vector of integers in
0–255.

## Colours

| Spelling | Example | Range |
|---|---|---|
| keyword | `:steelblue` | 35 named colours (CSS basics plus common extras) |
| hex string | `"#f80"`, `"#ff8800"`, `"#ff8800cc"` | 3, 4, 6 or 8 digits |
| integer vector | `[255 136 0]`, `[255 136 0 200]` | each channel 0–255 |
| float vector | `[1.0 0.53 0.0 0.8]` | each channel 0.0–1.0 |

A vector is read as integer channels when **every** element is a `Long`, and as
float channels as soon as one is a `Double`. So `[1 1 1]` is near-black and
`[1.0 1.0 1.0]` is white — the same distinction Clojure draws between `1` and
`1.0`.

`(color spec)` exposes this vocabulary as a function, so Clojure code can
normalize once and compute on the result rather than reimplementing the rules:

```clojure
(r/color :crimson)   ;=> [220 20 60 255]
(r/color "#f80")     ;=> [255 136 0 255]
```

## Options map

```clojure
{:color      :steelblue
 :gradient   {:type :linear            ; :linear | :radial — wins over :color
              :start [0 0] :end [100 0]
              :radius 40               ; :radial only
              :stops [[0.0 :white] [1.0 :navy]]
              :spread :pad}            ; :pad | :reflect | :repeat
 :anti-alias true
 :blend      :src-over                 ; the 29 Skia blend modes, kebab-cased
 :fill-rule  :winding                  ; :winding | :even-odd — fills only
 :transform  {:translate [10 10] :scale [2 2] :rotate 30 :around [50 50]}

 ;; strokes only
 :width       2.0
 :line-cap    :butt                    ; :butt | :round | :square
 :line-join   :miter                   ; :miter | :round | :bevel
 :miter-limit 4.0
 :dash        {:array [6 3] :offset 0}}
```

`:transform` composes **scale, then rotate, then translate**, with the rotation
pivoting on `:around` when given. A six-element vector `[sx ky kx sy tx ty]` sets
the matrix directly instead.

`draw-canvas!` additionally takes `:opacity` (0.0–1.0) and `:quality`
(`:nearest` | `:bilinear` | `:bicubic`).

## Usage

```clojure
(require '[cljrs.raster :as r])

(def c (r/canvas 320 180))
(r/clear! c :white)
(r/fill-rect! c 20 20 120 60 {:color :steelblue})
(r/fill-path! c (r/circle 240 90 45)
              {:gradient {:type :radial :start [240 90] :end [240 90] :radius 45
                          :stops [[0.0 :gold] [1.0 :crimson]]}})
(r/stroke-path! c (r/polyline [[10 160] [120 110] [310 165]])
                {:color :navy :width 4 :line-cap :round})
(r/save-png! c "/tmp/out.png")
```

## What this crate does not do

`tiny-skia` has **no text shaping or font rasterization**, so there is no
`draw-text!`. Adding one means pulling in a font stack (`fontdue`,
`cosmic-text`) with its own binary-size cost — a decision to make deliberately,
not to smuggle in behind a convenience function.

There is also no clipping-mask surface exposed yet (`tiny_skia::Mask` exists;
every draw call currently passes `None`).

## Features

| Feature | Default | Effect |
|---|---|---|
| `regex-full` | **on** | Forwards `regex-full` to this crate's workspace dependencies |
| `small-regex` | off | Forwards `small-regex` (`regex-lite`) instead |
| `deps` | **on** | Pass-through for `cljrs-runtime/deps` |

Every workspace dependency is taken with default features off (see the note in
the root `Cargo.toml`), so these pass-throughs put back what those crates'
defaults used to provide.

## Tests

```bash
cargo test -p cljrs-raster                     # the Clojure suite via the Rust harness
cargo test -p cljrs-raster -- --nocapture      # with the per-namespace totals
```
