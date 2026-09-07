//! The canvas: an RGBA pixel buffer you draw into, and the bytes you get out.
//!
//! A `Canvas` is an opaque `NativeObject` wrapping a `tiny_skia::Pixmap` behind
//! a `Mutex`, so the same canvas can be threaded through `->` and drawn into
//! from Clojure without data races. Every `…!` function returns the canvas it
//! was given.
//!
//! Pixels are stored **premultiplied** internally, which is what the compositor
//! wants, and handed out **straight** (un-premultiplied) by `pixel` and
//! `rgba-bytes`, which is what PNG encoders, `ffmpeg -pix_fmt rgba`, and human
//! intuition want. The conversion happens at the boundary so Clojure code never
//! has to know the difference.

use std::sync::Mutex;

use cljrs_gc::{GcPtr, MarkVisitor, Trace};
use cljrs_interop::{NativeObject, Value, gc_native_object};
use cljrs_value::PersistentVector;
use tiny_skia::{ColorU8, FilterQuality, IntSize, Pixmap, PixmapPaint, Rect, Transform};

use crate::args::{opt, opt_f32, opt_name, opts_map};
use crate::color;
use crate::paint;
use crate::path::as_path;

const POISONED: &str = "canvas lock poisoned by a panic in another thread";

/// An RGBA raster surface.
pub struct Canvas {
    inner: Mutex<Pixmap>,
}

impl std::fmt::Debug for Canvas {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.inner.lock() {
            Ok(p) => write!(f, "Canvas({}x{})", p.width(), p.height()),
            Err(_) => write!(f, "Canvas(poisoned)"),
        }
    }
}

impl NativeObject for Canvas {
    fn type_tag(&self) -> &str {
        "Canvas"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

// A Pixmap is a flat byte buffer — it holds no GC values.
impl Trace for Canvas {
    fn trace(&self, _: &mut MarkVisitor) {}
}

impl Canvas {
    /// Hand a freshly rendered pixmap to Clojure as a canvas. `svg` builds one
    /// this way after rasterizing a document.
    pub(crate) fn wrap(pixmap: Pixmap) -> Value {
        Value::NativeObject(gc_native_object(Canvas {
            inner: Mutex::new(pixmap),
        }))
    }

    fn with<R>(&self, f: impl FnOnce(&mut Pixmap) -> R) -> Result<R, String> {
        let mut guard = self.inner.lock().map_err(|_| POISONED.to_string())?;
        Ok(f(&mut guard))
    }

    fn read<R>(&self, f: impl FnOnce(&Pixmap) -> R) -> Result<R, String> {
        let guard = self.inner.lock().map_err(|_| POISONED.to_string())?;
        Ok(f(&guard))
    }
}

/// Downcast a `Value` to a canvas.
pub fn as_canvas(v: &Value) -> Result<&Canvas, String> {
    match v {
        Value::NativeObject(obj) => obj
            .get()
            .downcast_ref::<Canvas>()
            .ok_or_else(|| format!("expected a Canvas, got {}", obj.get().type_tag())),
        other => Err(format!("expected a Canvas, got {}", other.type_name())),
    }
}

/// Accept bytes in any of the shapes Clojurust uses for binary data.
pub fn to_bytes(v: &Value) -> Result<Vec<u8>, String> {
    match v {
        Value::ByteBlob(b) => Ok(b.to_vec()),
        Value::ByteArray(a) => {
            let guard = a.get().lock().map_err(|_| POISONED.to_string())?;
            Ok(guard.iter().map(|b| *b as u8).collect())
        }
        Value::Vector(vec) => vec
            .get()
            .iter()
            .map(|e| match e {
                Value::Long(n) if (0..=255).contains(n) => Ok(*n as u8),
                Value::Long(n) => Err(format!("byte out of range 0–255: {n}")),
                other => Err(format!(
                    "a byte vector holds integers 0–255, got {}",
                    other.type_name()
                )),
            })
            .collect(),
        other => Err(format!(
            "expected bytes (byte-blob, byte-array or integer vector), got {}",
            other.type_name()
        )),
    }
}

/// Bytes handed back to Clojure as a `ByteArray`.
///
/// `ByteBlob` would be the zero-copy choice, but core's `alength` / `aget` /
/// `vec` do not reach into one — a caller could not so much as measure the PNG
/// it was just handed. `ByteArray` is what `cljrs.base64/decode` returns and
/// what those functions understand, so bytes from this crate behave like every
/// other byte value in the runtime.
fn byte_array(bytes: Vec<u8>) -> Value {
    Value::ByteArray(GcPtr::new(Mutex::new(
        bytes.into_iter().map(|b| b as i8).collect::<Vec<i8>>(),
    )))
}

// ── Construction ──────────────────────────────────────────────────────────────

pub fn new_canvas(w: u32, h: u32) -> Result<Value, String> {
    Pixmap::new(w, h)
        .map(Canvas::wrap)
        .ok_or_else(|| format!("cannot allocate a {w}x{h} canvas"))
}

pub fn clone_canvas(c: &Value) -> Result<Value, String> {
    let copy = as_canvas(c)?.read(|p| p.clone())?;
    Ok(Canvas::wrap(copy))
}

pub fn width(c: &Value) -> Result<i64, String> {
    as_canvas(c)?.read(|p| p.width() as i64)
}

pub fn height(c: &Value) -> Result<i64, String> {
    as_canvas(c)?.read(|p| p.height() as i64)
}

// ── Whole-surface operations ──────────────────────────────────────────────────

pub fn clear(c: &Value, colour: &Value) -> Result<Value, String> {
    let col = color::parse(colour)?;
    as_canvas(c)?.with(|p| p.fill(col))?;
    Ok(c.clone())
}

pub fn pixel(c: &Value, x: u32, y: u32) -> Result<Value, String> {
    let found = as_canvas(c)?.read(|p| p.pixel(x, y))?;
    match found {
        Some(px) => Ok(Value::Vector(GcPtr::new(PersistentVector::from_iter(
            color::to_rgba_vec(px.demultiply()),
        )))),
        None => Ok(Value::Nil),
    }
}

pub fn set_pixel(c: &Value, x: u32, y: u32, colour: &Value) -> Result<Value, String> {
    let col = color::parse(colour)?.to_color_u8();
    let canvas = as_canvas(c)?;
    let ok = canvas.with(|p| {
        if x >= p.width() || y >= p.height() {
            return false;
        }
        let idx = (y * p.width() + x) as usize;
        p.pixels_mut()[idx] = col.premultiply();
        true
    })?;
    if !ok {
        return Err(format!("pixel ({x}, {y}) is outside the canvas"));
    }
    Ok(c.clone())
}

// ── Drawing ───────────────────────────────────────────────────────────────────

pub fn fill_rect(c: &Value, r: &[f32], opts: &Value) -> Result<Value, String> {
    let d = paint::parse(opts)?;
    let rect = Rect::from_xywh(r[0], r[1], r[2], r[3])
        .ok_or_else(|| format!("invalid rect x={} y={} w={} h={}", r[0], r[1], r[2], r[3]))?;
    as_canvas(c)?.with(|p| p.fill_rect(rect, &d.paint, d.transform, None))?;
    Ok(c.clone())
}

pub fn fill_path(c: &Value, path: &Value, opts: &Value) -> Result<Value, String> {
    let d = paint::parse(opts)?;
    let path = as_path(path)?;
    as_canvas(c)?.with(|p| p.fill_path(path, &d.paint, d.fill_rule, d.transform, None))?;
    Ok(c.clone())
}

pub fn stroke_path(c: &Value, path: &Value, opts: &Value) -> Result<Value, String> {
    let d = paint::parse(opts)?;
    let stroke = paint::parse_stroke(opts)?;
    let path = as_path(path)?;
    as_canvas(c)?.with(|p| p.stroke_path(path, &d.paint, &stroke, d.transform, None))?;
    Ok(c.clone())
}

/// Composite `src` onto `dst` at integer offset `(x, y)`.
///
/// Options: `:opacity` (0.0–1.0), `:blend`, `:quality`
/// (`:nearest` | `:bilinear` | `:bicubic`) and `:transform`.
pub fn draw_canvas(
    dst: &Value,
    src: &Value,
    x: i32,
    y: i32,
    opts: &Value,
) -> Result<Value, String> {
    let m = opts_map(opts)?;
    let mut pp = PixmapPaint {
        opacity: opt_f32(&m, "opacity")?.unwrap_or(1.0).clamp(0.0, 1.0),
        ..PixmapPaint::default()
    };
    if opt_name(&m, "blend")?.is_some() {
        // One blend-mode vocabulary for the whole crate: let the paint parser
        // own it rather than keeping a second table in sync here.
        pp.blend_mode = paint::parse(opts)?.paint.blend_mode;
    }
    if let Some(q) = opt_name(&m, "quality")? {
        pp.quality = match q.as_str() {
            "nearest" => FilterQuality::Nearest,
            "bilinear" => FilterQuality::Bilinear,
            "bicubic" => FilterQuality::Bicubic,
            other => return Err(format!("unknown filter quality :{other}")),
        };
    }
    let transform = match opt(&m, "transform") {
        Some(t) => paint::parse_transform(&t)?,
        None => Transform::identity(),
    };

    // Copy the source first: `dst` and `src` may be the same canvas, and the
    // Mutex is not reentrant.
    let source = as_canvas(src)?.read(|p| p.clone())?;
    as_canvas(dst)?.with(|p| p.draw_pixmap(x, y, source.as_ref(), &pp, transform, None))?;
    Ok(dst.clone())
}

// ── Encoding and raw pixels ───────────────────────────────────────────────────

pub fn encode_png(c: &Value) -> Result<Value, String> {
    let bytes = as_canvas(c)?.read(|p| p.encode_png())?;
    bytes.map(byte_array).map_err(|e| e.to_string())
}

pub fn save_png(c: &Value, path: &str) -> Result<String, String> {
    as_canvas(c)?
        .read(|p| p.save_png(path))?
        .map_err(|e| format!("{path}: {e}"))?;
    Ok(path.to_string())
}

pub fn load_png(path: &str) -> Result<Value, String> {
    Pixmap::load_png(path)
        .map(Canvas::wrap)
        .map_err(|e| format!("{path}: {e}"))
}

pub fn decode_png(bytes: &Value) -> Result<Value, String> {
    let bytes = to_bytes(bytes)?;
    Pixmap::decode_png(&bytes)
        .map(Canvas::wrap)
        .map_err(|e| e.to_string())
}

/// Straight (un-premultiplied) RGBA, row-major, 4 bytes per pixel.
///
/// This is the exact layout `ffmpeg -f rawvideo -pixel_format rgba` expects,
/// which is what makes `cljrs.ffmpeg/write-frame!` a one-liner.
pub fn rgba_bytes(c: &Value) -> Result<Value, String> {
    let bytes = as_canvas(c)?.read(|p| {
        let mut out = Vec::with_capacity(p.pixels().len() * 4);
        for px in p.pixels() {
            let s = px.demultiply();
            out.extend_from_slice(&[s.red(), s.green(), s.blue(), s.alpha()]);
        }
        out
    })?;
    Ok(byte_array(bytes))
}

/// The inverse of [`rgba_bytes`] — build a canvas from straight RGBA bytes.
pub fn from_rgba(w: u32, h: u32, bytes: &Value) -> Result<Value, String> {
    let bytes = to_bytes(bytes)?;
    let expected = (w as usize) * (h as usize) * 4;
    if bytes.len() != expected {
        return Err(format!(
            "expected {expected} bytes for a {w}x{h} RGBA image, got {}",
            bytes.len()
        ));
    }
    let size = IntSize::from_wh(w, h).ok_or_else(|| format!("invalid size {w}x{h}"))?;
    let mut pixmap = Pixmap::new(size.width(), size.height())
        .ok_or_else(|| format!("cannot allocate {w}x{h}"))?;
    for (i, px) in pixmap.pixels_mut().iter_mut().enumerate() {
        let b = &bytes[i * 4..i * 4 + 4];
        *px = ColorU8::from_rgba(b[0], b[1], b[2], b[3]).premultiply();
    }
    Ok(Canvas::wrap(pixmap))
}

/// Used by `cljrs-ffmpeg` to accept a canvas without re-deriving the layout.
pub fn canvas_rgba(v: &Value) -> Option<Result<Vec<u8>, String>> {
    as_canvas(v).ok()?;
    Some(rgba_bytes(v).and_then(|bytes| to_bytes(&bytes)))
}

/// The canvas dimensions, for callers outside this crate.
pub fn canvas_size(v: &Value) -> Result<(u32, u32), String> {
    as_canvas(v)?.read(|p| (p.width(), p.height()))
}
