# cljrs-ffmpeg

FFmpeg media probing, transcoding and frame streaming for Clojurust. Registers
the `cljrs.ffmpeg` namespace.

## Purpose

Let Clojure code inspect media files, transcode them, pull frames out of them,
and — the part that pairs with `cljrs-raster` — push rendered frames *into* a
live encoder, so an animation is a `doseq` that draws a canvas and writes it.

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

## Why the CLI and not `libav*`

This crate drives the `ffmpeg` and `ffprobe` **executables** rather than linking
the C libraries. That is a deliberate trade:

- the C libraries move fast and the Rust bindings lag (as of writing
  `libavcodec` is at 62 and the common binding crates cap well below that), and
  they need `clang` plus dev headers at build time — so `cargo build` of this
  workspace would start failing on machines that merely lack a package;
- the CLI is a stable, documented interface that every platform ships, and the
  process boundary keeps a codec bug out of the interpreter's address space: a
  segfault in a filter graph kills a child, not the REPL.

The cost is per-invocation process overhead and text parsing, which is
irrelevant for probing and transcoding, and avoided entirely for frame
streaming — `video-writer` holds **one** long-lived child and pipes raw frames
into its stdin.

## File layout

| File | Description |
|---|---|
| `src/lib.rs` | `NS`, `init`, `register`, `cljrs_init`; option parsing and every `registry.define` call |
| `src/proc.rs` | Binary discovery (`$CLJRS_FFMPEG`, `$CLJRS_FFPROBE`) and process running |
| `src/probe.rs` | `ffprobe` JSON → Clojure data; `probe`, `duration`, `dimensions` |
| `src/writer.rs` | The `VideoWriter` resource: spawn, `write_frame`, `finish` |
| `test/cljrs/ffmpeg_test.cljrs` | `clojure.test` suite that starts no process (8 tests, 18 assertions) |
| `test/cljrs/ffmpeg_encode_test.cljrs` | Suite that actually encodes, probes and extracts |
| `tests/clojure_tests.rs` | Rust harness; skips the encoding suite loudly when FFmpeg is absent |

## Public API

### Rust

```rust
/// Clojure namespace registered by this crate.
pub const NS: &str = "cljrs.ffmpeg";

pub fn init(globals: &Arc<GlobalEnv>);
pub fn register(registry: &mut Registry);

/// An ffmpeg child fed raw RGBA over stdin. A `Resource`, not a `NativeObject`:
/// it owns a process and a pipe, and the GC has no finalizers.
pub struct VideoWriter;
pub struct WriterSpec { /* width, height, fps, codec, pix_fmt, crf, preset, … */ }
pub struct Outcome { pub exit: i32, pub stderr: String, pub frames: u64 }
```

### Clojure namespace: `cljrs.ffmpeg`

#### Inspect

| Symbol | Signature | Returns |
|---|---|---|
| `available?` | `(available?)` | `Boolean` — both binaries runnable |
| `version` | `(version)` | The `ffmpeg -version` banner line |
| `probe` | `(probe path)` | `{:format {…} :streams [{…}]}` |
| `duration` | `(duration path)` | `Double` seconds |
| `dimensions` | `(dimensions path)` | `[width height]` of the first video stream |

#### One-shot

| Symbol | Signature | Returns |
|---|---|---|
| `run` | `(run ["-i" "in.mp4" "out.mp4"])` | `{:exit n :out "…" :error "…"}` |
| `extract-frame!` | `(extract-frame! in out [opts])` | the output path |
| `transcode!` | `(transcode! in out [opts])` | the output path |

`extract-frame!` options: `:time`, `:scale [w h]`, `:args`.
`transcode!` options: `:codec`, `:audio-codec`, `:crf`, `:preset`, `:fps`,
`:scale [w h]`, `:pix-fmt`, `:start`, `:duration`, `:loglevel`, `:overwrite`,
`:args`.

`run` fails only if the binary cannot be started; a non-zero FFmpeg exit comes
back in the map. `extract-frame!` and `transcode!` throw on a non-zero exit,
with the tail of stderr in the message.

#### Stream

| Symbol | Signature | Returns |
|---|---|---|
| `video-writer` | `(video-writer path opts)` | a `video-writer` resource |
| `write-frame!` | `(write-frame! w frame)` | the writer |
| `close-writer!` | `(close-writer! w)` | `{:path … :exit n :frames n :error nil}` |
| `writer-closed?` | `(writer-closed? w)` | `Boolean` |
| `frames-written` | `(frames-written w)` | `Long` |
| `writer?` | `(writer? v)` | `Boolean` |

`video-writer` options: `:width` and `:height` (**required**), `:fps` (30),
`:codec` (`"libx264"`), `:pix-fmt` (`"yuv420p"`), `:crf`, `:preset`,
`:loglevel` (`"error"`), `:overwrite` (true), `:args`.

A `frame` is straight-RGBA bytes: a `ByteArray`, a `ByteBlob`, or a vector of
integers in 0–255. That is exactly what `cljrs.raster/rgba-bytes` returns.

`close-writer!` is idempotent: a second call returns the same outcome rather
than waiting again.

## Two things the writer checks up front

- **Odd dimensions with a 4:2:0 pixel format are refused before spawning.** The
  chroma planes are half-resolution, so there is no valid encoding; FFmpeg's own
  error for this is buried far into stderr.
- **Every frame's byte count is verified.** `-f rawvideo` has no framing:
  ffmpeg slices stdin into `width*height*4`-byte chunks, so one short write
  shears every subsequent frame. A mismatch is refused rather than producing a
  video that is subtly wrong, and the refused write is not counted.

Its stderr is drained on a background thread — an unread pipe fills and blocks
the encoder mid-write, a deadlock that only shows up on long renders.

## Data conversion

`probe` returns ordinary Clojure data. JSON keys become keywords with `_`
rewritten to `-`, so `codec_name` reads as `:codec-name`. Values are left
exactly as ffprobe emits them — ffprobe reports many numbers as JSON *strings*
(`"duration": "12.345"`), and this crate does not guess which to coerce. Use
`duration` and `dimensions` for the two that matter; parse the rest yourself.

## Usage

```clojure
(require '[cljrs.ffmpeg :as ff] '[cljrs.raster :as r])

(ff/probe "clip.mp4")
(ff/extract-frame! "clip.mp4" "/tmp/f.png" {:time 3.5 :scale [640 -1]})

(let [w (ff/video-writer "/tmp/spin.mp4" {:width 640 :height 360 :fps 30})]
  (doseq [i (range 90)]
    (let [c (r/canvas 640 360)]
      (r/clear! c "#101820")
      (r/fill-circle! c (+ 320 (* 200 (Math/cos (/ i 14.0)))) 180 40 {:color :gold})
      (ff/write-frame! w (r/rgba-bytes c))))
  (ff/close-writer! w))
;=> {:path "/tmp/spin.mp4" :exit 0 :frames 90 :error nil}
```

## Environment

| Variable | Effect |
|---|---|
| `CLJRS_FFMPEG` | Path to the `ffmpeg` binary (default: `ffmpeg` on `PATH`) |
| `CLJRS_FFPROBE` | Path to the `ffprobe` binary (default: `ffprobe` on `PATH`) |

## Features

| Feature | Default | Effect |
|---|---|---|
| `regex-full` | **on** | Forwards `regex-full` to this crate's workspace dependencies |
| `small-regex` | off | Forwards `small-regex` (`regex-lite`) instead |
| `deps` | **on** | Pass-through for `cljrs-runtime/deps` |

## Why this crate does not depend on `cljrs-raster`

They compose in Clojure, through `rgba-bytes`, not in Cargo:

```clojure
(ff/write-frame! w (r/rgba-bytes canvas))
```

An encoder that depends on a rasterizer is a layering inversion: an embedding
that wants `cljrs.ffmpeg` without `tiny-skia` should not have to pay for one,
and a `raster` feature flag to make that optional would be permanent
maintenance surface. The real contract between the two is the **byte layout**,
and making it explicit at the call site says so.

Passing a canvas straight to `write-frame!` is the obvious mistake, so it is
detected by type tag (which needs no dependency) and answered with an error that
names the conversion.

## Tests

```bash
cargo test -p cljrs-ffmpeg -- --nocapture
```

Without `ffmpeg` and `ffprobe` on the PATH the encoding namespace is skipped and
the harness says so; it does not report a pass for a suite it never ran.

The encoding suite builds its frames from byte vectors, since this crate has no
rasterizer. The canvas-to-video pipeline is covered where both namespaces really
coexist — the `cljrs` binary — by `samples/raster_video.cljrs`.
