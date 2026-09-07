//! 2D vector rasterization for Clojurust, backed by [`tiny_skia`].
//!
//! Registers the `cljrs.raster` namespace: an anti-aliased software rasterizer
//! with paths, fills, strokes, gradients, blend modes, affine transforms, PNG
//! encode/decode, and raw RGBA in and out.
//!
//! ```clojure
//! (require '[cljrs.raster :as r])
//!
//! (def c (r/canvas 320 180))
//! (r/clear! c :white)
//! (r/fill-rect! c 20 20 120 60 {:color :steelblue})
//! (r/fill-path! c (r/circle 220 90 50)
//!               {:gradient {:type :radial :start [220 90] :end [220 90]
//!                           :radius 50 :stops [[0.0 :gold] [1.0 :crimson]]}})
//! (r/stroke-path! c (r/polygon [[20 160] [160 100] [300 160]])
//!                 {:color :navy :width 3 :line-join :round})
//! (r/save-png! c "/tmp/out.png")
//! ```
//!
//! The straight-RGBA bytes from `rgba-bytes` are exactly what
//! `cljrs.ffmpeg/write-frame!` pipes to an encoder, so an animation is a
//! `doseq` that draws a canvas and writes `(r/rgba-bytes c)`. The two crates
//! compose through that byte layout in Clojure, not through a Cargo edge.
//!
//! ## What this crate deliberately does not do
//!
//! `tiny_skia` is a port of a Skia subset and has **no text shaping or font
//! rasterization**. There is no `draw-text!` here, and adding one means
//! pulling in a font stack (`fontdue` / `cosmic-text`) — a separate decision
//! with its own binary-size cost, not something to smuggle in behind a
//! convenience function.

use std::sync::Arc;

use cljrs_interop::{Registry, Value, wrap_fn_variadic, wrap_fn0, wrap_fn1, wrap_fn2, wrap_fn3};
use cljrs_runtime::env::env::GlobalEnv;

pub mod args;
pub mod canvas;
pub mod color;
pub mod paint;
pub mod path;

pub use canvas::{Canvas, as_canvas, canvas_rgba, canvas_size};
pub use path::{RasterPath, RasterPathBuilder};

/// The Clojure namespace this crate registers.
pub const NS: &str = "cljrs.raster";

/// The trailing options argument, or `nil` when the caller omitted it.
fn opts_at(args: &[Value], i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Nil)
}

/// Read `n` numbers starting at `args[from]`, requiring at least that many.
fn nums(args: &[Value], from: usize, n: usize, what: &str) -> Result<Vec<f32>, String> {
    if args.len() < from + n {
        return Err(format!(
            "{what} needs at least {} argument(s), got {}",
            from + n,
            args.len()
        ));
    }
    args[from..from + n].iter().map(args::as_f32).collect()
}

/// Register the `cljrs.raster` namespace into `globals`. Idempotent.
pub fn init(globals: &Arc<GlobalEnv>) {
    if globals.is_loaded(NS) {
        return;
    }
    globals.get_or_create_ns(NS);
    globals.refer_core(NS);
    let mut registry = Registry::for_require(globals.clone());
    register(&mut registry);
}

/// Register every `cljrs.raster` function through an existing `Registry`.
pub fn register(registry: &mut Registry) {
    register_canvas(registry);
    register_drawing(registry);
    register_paths(registry);
    register_codec(registry);
    // A native namespace has no source file, so `require` would go looking on
    // the source path and fail. Marking it loaded is what makes
    // `(require '[cljrs.raster :as r])` resolve.
    registry.env().mark_loaded(NS);
}

// ── Canvas lifecycle ──────────────────────────────────────────────────────────

fn register_canvas(registry: &mut Registry) {
    registry.define(
        "cljrs.raster/canvas",
        wrap_fn2(
            "cljrs.raster/canvas",
            |w: Value, h: Value| -> Result<Value, String> {
                canvas::new_canvas(args::as_u32(&w)?, args::as_u32(&h)?)
            },
        ),
    );

    registry.define(
        "cljrs.raster/canvas?",
        wrap_fn1("cljrs.raster/canvas?", |v: Value| -> Result<bool, String> {
            Ok(canvas::as_canvas(&v).is_ok())
        }),
    );

    registry.define(
        "cljrs.raster/width",
        wrap_fn1("cljrs.raster/width", |c: Value| -> Result<i64, String> {
            canvas::width(&c)
        }),
    );

    registry.define(
        "cljrs.raster/height",
        wrap_fn1("cljrs.raster/height", |c: Value| -> Result<i64, String> {
            canvas::height(&c)
        }),
    );

    registry.define(
        "cljrs.raster/clone-canvas",
        wrap_fn1(
            "cljrs.raster/clone-canvas",
            |c: Value| -> Result<Value, String> { canvas::clone_canvas(&c) },
        ),
    );

    registry.define(
        "cljrs.raster/clear!",
        wrap_fn2(
            "cljrs.raster/clear!",
            |c: Value, colour: Value| -> Result<Value, String> { canvas::clear(&c, &colour) },
        ),
    );

    registry.define(
        "cljrs.raster/pixel",
        wrap_fn3(
            "cljrs.raster/pixel",
            |c: Value, x: Value, y: Value| -> Result<Value, String> {
                canvas::pixel(&c, args::as_u32(&x)?, args::as_u32(&y)?)
            },
        ),
    );

    registry.define(
        "cljrs.raster/set-pixel!",
        wrap_fn_variadic(
            "cljrs.raster/set-pixel!",
            4,
            |a: &[Value]| -> Result<Value, String> {
                canvas::set_pixel(&a[0], args::as_u32(&a[1])?, args::as_u32(&a[2])?, &a[3])
            },
        ),
    );
}

// ── Drawing ───────────────────────────────────────────────────────────────────

fn register_drawing(registry: &mut Registry) {
    registry.define(
        "cljrs.raster/fill-rect!",
        wrap_fn_variadic(
            "cljrs.raster/fill-rect!",
            5,
            |a: &[Value]| -> Result<Value, String> {
                let r = nums(a, 1, 4, "fill-rect!")?;
                canvas::fill_rect(&a[0], &r, &opts_at(a, 5))
            },
        ),
    );

    registry.define(
        "cljrs.raster/stroke-rect!",
        wrap_fn_variadic(
            "cljrs.raster/stroke-rect!",
            5,
            |a: &[Value]| -> Result<Value, String> {
                let r = nums(a, 1, 4, "stroke-rect!")?;
                let p = path::rect_path(r[0], r[1], r[2], r[3])?;
                canvas::stroke_path(&a[0], &p, &opts_at(a, 5))
            },
        ),
    );

    registry.define(
        "cljrs.raster/fill-circle!",
        wrap_fn_variadic(
            "cljrs.raster/fill-circle!",
            4,
            |a: &[Value]| -> Result<Value, String> {
                let n = nums(a, 1, 3, "fill-circle!")?;
                let p = path::circle_path(n[0], n[1], n[2])?;
                canvas::fill_path(&a[0], &p, &opts_at(a, 4))
            },
        ),
    );

    registry.define(
        "cljrs.raster/stroke-circle!",
        wrap_fn_variadic(
            "cljrs.raster/stroke-circle!",
            4,
            |a: &[Value]| -> Result<Value, String> {
                let n = nums(a, 1, 3, "stroke-circle!")?;
                let p = path::circle_path(n[0], n[1], n[2])?;
                canvas::stroke_path(&a[0], &p, &opts_at(a, 4))
            },
        ),
    );

    registry.define(
        "cljrs.raster/stroke-line!",
        wrap_fn_variadic(
            "cljrs.raster/stroke-line!",
            5,
            |a: &[Value]| -> Result<Value, String> {
                let n = nums(a, 1, 4, "stroke-line!")?;
                let b = path::new_builder();
                path::move_to(&b, n[0], n[1])?;
                path::line_to(&b, n[2], n[3])?;
                let p = path::finish(&b)?;
                canvas::stroke_path(&a[0], &p, &opts_at(a, 5))
            },
        ),
    );

    registry.define(
        "cljrs.raster/fill-path!",
        wrap_fn_variadic(
            "cljrs.raster/fill-path!",
            2,
            |a: &[Value]| -> Result<Value, String> {
                canvas::fill_path(&a[0], &a[1], &opts_at(a, 2))
            },
        ),
    );

    registry.define(
        "cljrs.raster/stroke-path!",
        wrap_fn_variadic(
            "cljrs.raster/stroke-path!",
            2,
            |a: &[Value]| -> Result<Value, String> {
                canvas::stroke_path(&a[0], &a[1], &opts_at(a, 2))
            },
        ),
    );

    registry.define(
        "cljrs.raster/draw-canvas!",
        wrap_fn_variadic(
            "cljrs.raster/draw-canvas!",
            4,
            |a: &[Value]| -> Result<Value, String> {
                canvas::draw_canvas(
                    &a[0],
                    &a[1],
                    args::as_i32(&a[2])?,
                    args::as_i32(&a[3])?,
                    &opts_at(a, 4),
                )
            },
        ),
    );
}

// ── Paths ─────────────────────────────────────────────────────────────────────

fn register_paths(registry: &mut Registry) {
    registry.define(
        "cljrs.raster/path-builder",
        wrap_fn0("cljrs.raster/path-builder", || -> Result<Value, String> {
            Ok(path::new_builder())
        }),
    );

    registry.define(
        "cljrs.raster/move-to!",
        wrap_fn3(
            "cljrs.raster/move-to!",
            |b: Value, x: Value, y: Value| -> Result<Value, String> {
                path::move_to(&b, args::as_f32(&x)?, args::as_f32(&y)?)
            },
        ),
    );

    registry.define(
        "cljrs.raster/line-to!",
        wrap_fn3(
            "cljrs.raster/line-to!",
            |b: Value, x: Value, y: Value| -> Result<Value, String> {
                path::line_to(&b, args::as_f32(&x)?, args::as_f32(&y)?)
            },
        ),
    );

    registry.define(
        "cljrs.raster/quad-to!",
        wrap_fn_variadic(
            "cljrs.raster/quad-to!",
            5,
            |a: &[Value]| -> Result<Value, String> {
                path::quad_to(&a[0], &path::floats(a, 1, 4, "quad-to!")?)
            },
        ),
    );

    registry.define(
        "cljrs.raster/cubic-to!",
        wrap_fn_variadic(
            "cljrs.raster/cubic-to!",
            7,
            |a: &[Value]| -> Result<Value, String> {
                path::cubic_to(&a[0], &path::floats(a, 1, 6, "cubic-to!")?)
            },
        ),
    );

    registry.define(
        "cljrs.raster/close-path!",
        wrap_fn1(
            "cljrs.raster/close-path!",
            |b: Value| -> Result<Value, String> { path::close(&b) },
        ),
    );

    registry.define(
        "cljrs.raster/push-rect!",
        wrap_fn_variadic(
            "cljrs.raster/push-rect!",
            5,
            |a: &[Value]| -> Result<Value, String> {
                let r = path::floats(a, 1, 4, "push-rect!")?;
                path::push_rect(&a[0], r[0], r[1], r[2], r[3])
            },
        ),
    );

    registry.define(
        "cljrs.raster/push-oval!",
        wrap_fn_variadic(
            "cljrs.raster/push-oval!",
            5,
            |a: &[Value]| -> Result<Value, String> {
                let r = path::floats(a, 1, 4, "push-oval!")?;
                path::push_oval(&a[0], r[0], r[1], r[2], r[3])
            },
        ),
    );

    registry.define(
        "cljrs.raster/push-circle!",
        wrap_fn_variadic(
            "cljrs.raster/push-circle!",
            4,
            |a: &[Value]| -> Result<Value, String> {
                let n = path::floats(a, 1, 3, "push-circle!")?;
                path::push_circle(&a[0], n[0], n[1], n[2])
            },
        ),
    );

    registry.define(
        "cljrs.raster/finish-path",
        wrap_fn1(
            "cljrs.raster/finish-path",
            |b: Value| -> Result<Value, String> { path::finish(&b) },
        ),
    );

    registry.define(
        "cljrs.raster/path?",
        wrap_fn1("cljrs.raster/path?", |v: Value| -> Result<bool, String> {
            Ok(path::as_path(&v).is_ok())
        }),
    );

    registry.define(
        "cljrs.raster/rect",
        wrap_fn_variadic(
            "cljrs.raster/rect",
            4,
            |a: &[Value]| -> Result<Value, String> {
                let r = path::floats(a, 0, 4, "rect")?;
                path::rect_path(r[0], r[1], r[2], r[3])
            },
        ),
    );

    registry.define(
        "cljrs.raster/oval",
        wrap_fn_variadic(
            "cljrs.raster/oval",
            4,
            |a: &[Value]| -> Result<Value, String> {
                let r = path::floats(a, 0, 4, "oval")?;
                path::oval_path(r[0], r[1], r[2], r[3])
            },
        ),
    );

    registry.define(
        "cljrs.raster/circle",
        wrap_fn3(
            "cljrs.raster/circle",
            |cx: Value, cy: Value, r: Value| -> Result<Value, String> {
                path::circle_path(args::as_f32(&cx)?, args::as_f32(&cy)?, args::as_f32(&r)?)
            },
        ),
    );

    registry.define(
        "cljrs.raster/polygon",
        wrap_fn1(
            "cljrs.raster/polygon",
            |pts: Value| -> Result<Value, String> { path::poly_path(&pts, true) },
        ),
    );

    registry.define(
        "cljrs.raster/polyline",
        wrap_fn1(
            "cljrs.raster/polyline",
            |pts: Value| -> Result<Value, String> { path::poly_path(&pts, false) },
        ),
    );
}

// ── Encoding ──────────────────────────────────────────────────────────────────

fn register_codec(registry: &mut Registry) {
    registry.define(
        "cljrs.raster/encode-png",
        wrap_fn1(
            "cljrs.raster/encode-png",
            |c: Value| -> Result<Value, String> { canvas::encode_png(&c) },
        ),
    );

    registry.define(
        "cljrs.raster/save-png!",
        wrap_fn2(
            "cljrs.raster/save-png!",
            |c: Value, p: String| -> Result<String, String> { canvas::save_png(&c, &p) },
        ),
    );

    registry.define(
        "cljrs.raster/load-png",
        wrap_fn1(
            "cljrs.raster/load-png",
            |p: String| -> Result<Value, String> { canvas::load_png(&p) },
        ),
    );

    registry.define(
        "cljrs.raster/decode-png",
        wrap_fn1(
            "cljrs.raster/decode-png",
            |b: Value| -> Result<Value, String> { canvas::decode_png(&b) },
        ),
    );

    registry.define(
        "cljrs.raster/rgba-bytes",
        wrap_fn1(
            "cljrs.raster/rgba-bytes",
            |c: Value| -> Result<Value, String> { canvas::rgba_bytes(&c) },
        ),
    );

    registry.define(
        "cljrs.raster/from-rgba",
        wrap_fn3(
            "cljrs.raster/from-rgba",
            |w: Value, h: Value, b: Value| -> Result<Value, String> {
                canvas::from_rgba(args::as_u32(&w)?, args::as_u32(&h)?, &b)
            },
        ),
    );
}
