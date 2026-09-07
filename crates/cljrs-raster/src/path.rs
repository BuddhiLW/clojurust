//! Paths: an incremental builder and the immutable path it produces.
//!
//! Two opaque `NativeObject`s live here. A `RasterPathBuilder` accumulates
//! segments and is consumed by `finish-path`; a `RasterPath` is the immutable
//! result that `fill-path!` and `stroke-path!` draw. Keeping them distinct is
//! what makes a finished path safe to hold onto and draw repeatedly — a
//! builder is single-use by construction, so there is no way to mutate a path
//! another frame is still drawing.
//!
//! ```clojure
//! (-> (r/path-builder)
//!     (r/move-to! 10 10)
//!     (r/line-to! 90 10)
//!     (r/line-to! 50 80)
//!     (r/close-path!)
//!     (r/finish-path))
//! ```

use std::sync::Mutex;

use cljrs_gc::{MarkVisitor, Trace};
use cljrs_interop::{NativeObject, Value, gc_native_object};
use tiny_skia::{Path, PathBuilder, Rect};

use crate::args::{as_f32, as_points};

// ── The two native objects ────────────────────────────────────────────────────

/// An in-progress path. `finish-path` takes the builder out, leaving the
/// object spent; every later call on it errors rather than silently drawing
/// nothing.
pub struct RasterPathBuilder {
    inner: Mutex<Option<PathBuilder>>,
}

impl std::fmt::Debug for RasterPathBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PathBuilder").finish_non_exhaustive()
    }
}

impl NativeObject for RasterPathBuilder {
    fn type_tag(&self) -> &str {
        "PathBuilder"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

// Holds no GC values.
impl Trace for RasterPathBuilder {
    fn trace(&self, _: &mut MarkVisitor) {}
}

impl RasterPathBuilder {
    fn new() -> Self {
        Self {
            inner: Mutex::new(Some(PathBuilder::new())),
        }
    }

    /// Run `f` against the live builder.
    fn with<R>(&self, f: impl FnOnce(&mut PathBuilder) -> R) -> Result<R, String> {
        let mut guard = self.inner.lock().map_err(|_| POISONED.to_string())?;
        match guard.as_mut() {
            Some(b) => Ok(f(b)),
            None => Err("this path builder has already been finished".to_string()),
        }
    }

    fn take(&self) -> Result<PathBuilder, String> {
        let mut guard = self.inner.lock().map_err(|_| POISONED.to_string())?;
        guard
            .take()
            .ok_or_else(|| "this path builder has already been finished".to_string())
    }
}

/// A finished, immutable path.
#[derive(Debug)]
pub struct RasterPath {
    pub path: Path,
}

impl NativeObject for RasterPath {
    fn type_tag(&self) -> &str {
        "Path"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl Trace for RasterPath {
    fn trace(&self, _: &mut MarkVisitor) {}
}

const POISONED: &str = "path lock poisoned by a panic in another thread";

// ── Downcasting ───────────────────────────────────────────────────────────────

pub fn as_builder(v: &Value) -> Result<&RasterPathBuilder, String> {
    match v {
        Value::NativeObject(obj) => obj
            .get()
            .downcast_ref::<RasterPathBuilder>()
            .ok_or_else(|| format!("expected a PathBuilder, got {}", obj.get().type_tag())),
        other => Err(format!("expected a PathBuilder, got {}", other.type_name())),
    }
}

pub fn as_path(v: &Value) -> Result<&Path, String> {
    match v {
        Value::NativeObject(obj) => obj
            .get()
            .downcast_ref::<RasterPath>()
            .map(|p| &p.path)
            .ok_or_else(|| format!("expected a Path, got {}", obj.get().type_tag())),
        other => Err(format!("expected a Path, got {}", other.type_name())),
    }
}

// ── Constructors used by the registered functions ─────────────────────────────

pub fn new_builder() -> Value {
    Value::NativeObject(gc_native_object(RasterPathBuilder::new()))
}

pub fn wrap_path(path: Path) -> Value {
    Value::NativeObject(gc_native_object(RasterPath { path }))
}

fn rect_of(x: f32, y: f32, w: f32, h: f32) -> Result<Rect, String> {
    Rect::from_xywh(x, y, w, h)
        .ok_or_else(|| format!("invalid rect x={x} y={y} width={w} height={h}"))
}

// ── Builder mutations ─────────────────────────────────────────────────────────
//
// Each returns the builder value it was handed, so calls thread through `->`.

pub fn move_to(b: &Value, x: f32, y: f32) -> Result<Value, String> {
    as_builder(b)?.with(|pb| pb.move_to(x, y))?;
    Ok(b.clone())
}

pub fn line_to(b: &Value, x: f32, y: f32) -> Result<Value, String> {
    as_builder(b)?.with(|pb| pb.line_to(x, y))?;
    Ok(b.clone())
}

pub fn quad_to(b: &Value, args: &[f32]) -> Result<Value, String> {
    as_builder(b)?.with(|pb| pb.quad_to(args[0], args[1], args[2], args[3]))?;
    Ok(b.clone())
}

pub fn cubic_to(b: &Value, a: &[f32]) -> Result<Value, String> {
    as_builder(b)?.with(|pb| pb.cubic_to(a[0], a[1], a[2], a[3], a[4], a[5]))?;
    Ok(b.clone())
}

pub fn close(b: &Value) -> Result<Value, String> {
    as_builder(b)?.with(|pb| pb.close())?;
    Ok(b.clone())
}

pub fn push_rect(b: &Value, x: f32, y: f32, w: f32, h: f32) -> Result<Value, String> {
    let r = rect_of(x, y, w, h)?;
    as_builder(b)?.with(|pb| pb.push_rect(r))?;
    Ok(b.clone())
}

pub fn push_oval(b: &Value, x: f32, y: f32, w: f32, h: f32) -> Result<Value, String> {
    let r = rect_of(x, y, w, h)?;
    as_builder(b)?.with(|pb| pb.push_oval(r))?;
    Ok(b.clone())
}

pub fn push_circle(b: &Value, cx: f32, cy: f32, r: f32) -> Result<Value, String> {
    as_builder(b)?.with(|pb| pb.push_circle(cx, cy, r))?;
    Ok(b.clone())
}

pub fn finish(b: &Value) -> Result<Value, String> {
    let pb = as_builder(b)?.take()?;
    let path = pb
        .finish()
        .ok_or_else(|| "cannot finish an empty or degenerate path".to_string())?;
    Ok(wrap_path(path))
}

// ── One-shot path constructors ────────────────────────────────────────────────

pub fn rect_path(x: f32, y: f32, w: f32, h: f32) -> Result<Value, String> {
    Ok(wrap_path(PathBuilder::from_rect(rect_of(x, y, w, h)?)))
}

pub fn oval_path(x: f32, y: f32, w: f32, h: f32) -> Result<Value, String> {
    PathBuilder::from_oval(rect_of(x, y, w, h)?)
        .map(wrap_path)
        .ok_or_else(|| format!("invalid oval x={x} y={y} width={w} height={h}"))
}

pub fn circle_path(cx: f32, cy: f32, r: f32) -> Result<Value, String> {
    PathBuilder::from_circle(cx, cy, r)
        .map(wrap_path)
        .ok_or_else(|| format!("invalid circle radius {r}"))
}

/// `closed` distinguishes `polygon` (closed) from `polyline` (open).
pub fn poly_path(points: &Value, closed: bool) -> Result<Value, String> {
    let pts = as_points(points)?;
    if pts.len() < 2 {
        return Err("a polyline needs at least two points".to_string());
    }
    let mut pb = PathBuilder::new();
    pb.move_to(pts[0].0, pts[0].1);
    for (x, y) in &pts[1..] {
        pb.line_to(*x, *y);
    }
    if closed {
        pb.close();
    }
    pb.finish()
        .map(wrap_path)
        .ok_or_else(|| "degenerate polygon".to_string())
}

/// Read `n` trailing numeric arguments out of a variadic call.
pub fn floats(args: &[Value], from: usize, n: usize, what: &str) -> Result<Vec<f32>, String> {
    if args.len() != from + n {
        return Err(format!(
            "{what} takes {} argument(s), got {}",
            from + n,
            args.len()
        ));
    }
    args[from..].iter().map(as_f32).collect()
}
