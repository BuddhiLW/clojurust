//! Options maps → `tiny_skia` paint, stroke, fill-rule and transform.
//!
//! Every drawing function takes one trailing options map, so a single vocabulary
//! covers fills, strokes and compositing:
//!
//! ```clojure
//! {:color      :steelblue          ; or "#4682b4" or [70 130 180]
//!  :gradient   {:type :linear      ; :linear or :radial — wins over :color
//!               :start [0 0] :end [100 0]
//!               :radius 40         ; :radial only
//!               :stops [[0.0 :white] [1.0 :navy]]
//!               :spread :pad}      ; :pad | :reflect | :repeat
//!  :anti-alias true
//!  :blend      :src-over
//!  :fill-rule  :winding            ; :winding or :even-odd — fills only
//!  :transform  {:translate [10 10] :scale [2 2] :rotate 30 :around [50 50]}
//!  ;; strokes only
//!  :width       2.0
//!  :line-cap    :butt              ; :butt | :round | :square
//!  :line-join   :miter             ; :miter | :round | :bevel
//!  :miter-limit 4.0
//!  :dash        {:array [6 3] :offset 0}}
//! ```
//!
//! A `:transform` map composes **scale, then rotate, then translate**; the
//! rotation pivots on `:around` when given and on the origin otherwise. Pass a
//! six-element vector `[sx ky kx sy tx ty]` instead to set the matrix directly.

use cljrs_value::Value;
use tiny_skia::{
    BlendMode, Color, FillRule, GradientStop, LineCap, LineJoin, LinearGradient, Paint, Point,
    RadialGradient, Shader, SpreadMode, Stroke, StrokeDash, Transform,
};

use crate::args::{as_coords, as_f32, as_f32_vec, opt, opt_bool, opt_f32, opt_name, opts_map};
use crate::color;

/// Everything a fill or stroke needs, parsed once from one options map.
pub struct DrawOpts {
    pub paint: Paint<'static>,
    pub fill_rule: FillRule,
    pub transform: Transform,
}

fn blend_mode(name: &str) -> Result<BlendMode, String> {
    Ok(match name {
        "clear" => BlendMode::Clear,
        "src" | "source" => BlendMode::Source,
        "dst" | "destination" => BlendMode::Destination,
        "src-over" | "source-over" => BlendMode::SourceOver,
        "dst-over" | "destination-over" => BlendMode::DestinationOver,
        "src-in" | "source-in" => BlendMode::SourceIn,
        "dst-in" | "destination-in" => BlendMode::DestinationIn,
        "src-out" | "source-out" => BlendMode::SourceOut,
        "dst-out" | "destination-out" => BlendMode::DestinationOut,
        "src-atop" | "source-atop" => BlendMode::SourceAtop,
        "dst-atop" | "destination-atop" => BlendMode::DestinationAtop,
        "xor" => BlendMode::Xor,
        "plus" => BlendMode::Plus,
        "modulate" => BlendMode::Modulate,
        "screen" => BlendMode::Screen,
        "overlay" => BlendMode::Overlay,
        "darken" => BlendMode::Darken,
        "lighten" => BlendMode::Lighten,
        "color-dodge" => BlendMode::ColorDodge,
        "color-burn" => BlendMode::ColorBurn,
        "hard-light" => BlendMode::HardLight,
        "soft-light" => BlendMode::SoftLight,
        "difference" => BlendMode::Difference,
        "exclusion" => BlendMode::Exclusion,
        "multiply" => BlendMode::Multiply,
        "hue" => BlendMode::Hue,
        "saturation" => BlendMode::Saturation,
        "color" => BlendMode::Color,
        "luminosity" => BlendMode::Luminosity,
        other => return Err(format!("unknown blend mode :{other}")),
    })
}

fn spread_mode(name: &str) -> Result<SpreadMode, String> {
    Ok(match name {
        "pad" => SpreadMode::Pad,
        "reflect" => SpreadMode::Reflect,
        "repeat" => SpreadMode::Repeat,
        other => return Err(format!("unknown gradient spread :{other}")),
    })
}

fn line_cap(name: &str) -> Result<LineCap, String> {
    Ok(match name {
        "butt" => LineCap::Butt,
        "round" => LineCap::Round,
        "square" => LineCap::Square,
        other => return Err(format!("unknown line cap :{other}")),
    })
}

fn line_join(name: &str) -> Result<LineJoin, String> {
    Ok(match name {
        "miter" => LineJoin::Miter,
        "round" => LineJoin::Round,
        "bevel" => LineJoin::Bevel,
        other => return Err(format!("unknown line join :{other}")),
    })
}

fn fill_rule(name: &str) -> Result<FillRule, String> {
    Ok(match name {
        "winding" | "non-zero" | "nonzero" => FillRule::Winding,
        "even-odd" | "evenodd" => FillRule::EvenOdd,
        other => return Err(format!("unknown fill rule :{other}")),
    })
}

fn point(v: &Value, what: &str) -> Result<Point, String> {
    let xy = as_coords(v, 2, what)?;
    Ok(Point::from_xy(xy[0], xy[1]))
}

fn stops(v: &Value) -> Result<Vec<GradientStop>, String> {
    let items = match v {
        Value::Vector(vec) => vec.get().iter().collect::<Vec<_>>(),
        other => {
            return Err(format!(
                ":stops must be a vector of [position colour] pairs, got {}",
                other.type_name()
            ));
        }
    };
    if items.is_empty() {
        return Err(":stops must hold at least one [position colour] pair".to_string());
    }
    items
        .iter()
        .map(|pair| match pair {
            Value::Vector(p) if p.get().count() == 2 => {
                let p = p.get();
                let pos = as_f32(p.nth(0).expect("count checked"))?;
                let col = color::parse(p.nth(1).expect("count checked"))?;
                Ok(GradientStop::new(pos, col))
            }
            other => Err(format!(
                "each gradient stop must be [position colour], got {}",
                other.type_name()
            )),
        })
        .collect()
}

/// Build a gradient shader from a `:gradient` sub-map.
fn gradient(v: &Value) -> Result<Shader<'static>, String> {
    let m = opts_map(v)?;
    let kind = opt_name(&m, "type")?.unwrap_or_else(|| "linear".to_string());
    let spread = spread_mode(&opt_name(&m, "spread")?.unwrap_or_else(|| "pad".to_string()))?;
    let stops = stops(&opt(&m, "stops").ok_or_else(|| "a gradient needs :stops".to_string())?)?;
    let start = point(
        &opt(&m, "start").ok_or_else(|| "a gradient needs :start".to_string())?,
        ":start",
    )?;
    let end = point(
        &opt(&m, "end").ok_or_else(|| "a gradient needs :end".to_string())?,
        ":end",
    )?;
    let transform = match opt(&m, "transform") {
        Some(Value::Nil) | None => Transform::identity(),
        Some(t) => parse_transform(&t)?,
    };

    match kind.as_str() {
        "linear" => LinearGradient::new(start, end, stops, spread, transform)
            .ok_or_else(|| "degenerate linear gradient (zero length or no stops)".to_string()),
        "radial" => {
            let radius = opt_f32(&m, "radius")?
                .ok_or_else(|| "a :radial gradient needs :radius".to_string())?;
            RadialGradient::new(start, end, radius, stops, spread, transform)
                .ok_or_else(|| "degenerate radial gradient (radius must be > 0)".to_string())
        }
        other => Err(format!("unknown gradient type :{other}")),
    }
}

/// Parse a `:transform` value: a map of named steps, or a six-element matrix.
pub fn parse_transform(v: &Value) -> Result<Transform, String> {
    match v {
        Value::Nil => Ok(Transform::identity()),
        Value::Vector(vec) if vec.get().count() == 6 => {
            let m = as_f32_vec(v, ":transform")?;
            Ok(Transform::from_row(m[0], m[1], m[2], m[3], m[4], m[5]))
        }
        Value::Map(_) => {
            let m = opts_map(v)?;
            // Scale, then rotate, then translate: the order that makes
            // `{:scale [2 2] :rotate 30}` mean "twice as big, turned 30°".
            let mut t = Transform::identity();
            if let Some(s) = opt(&m, "scale") {
                let s = match &s {
                    Value::Vector(_) => as_coords(&s, 2, ":scale")?,
                    other => {
                        let n = as_f32(other)?;
                        vec![n, n]
                    }
                };
                t = t.post_concat(Transform::from_scale(s[0], s[1]));
            }
            if let Some(deg) = opt_f32(&m, "rotate")? {
                let r = match opt(&m, "around") {
                    Some(p) if !matches!(p, Value::Nil) => {
                        let xy = as_coords(&p, 2, ":around")?;
                        Transform::from_rotate_at(deg, xy[0], xy[1])
                    }
                    _ => Transform::from_rotate(deg),
                };
                t = t.post_concat(r);
            }
            if let Some(tr) = opt(&m, "translate") {
                let xy = as_coords(&tr, 2, ":translate")?;
                t = t.post_concat(Transform::from_translate(xy[0], xy[1]));
            }
            Ok(t)
        }
        other => Err(format!(
            ":transform must be a map or a 6-element matrix vector, got {}",
            other.type_name()
        )),
    }
}

/// Parse the shared paint options: colour or gradient, anti-aliasing, blend
/// mode, fill rule and transform.
pub fn parse(opts: &Value) -> Result<DrawOpts, String> {
    let m = opts_map(opts)?;
    let shader = match opt(&m, "gradient") {
        Some(g) if !matches!(g, Value::Nil) => gradient(&g)?,
        _ => match opt(&m, "color") {
            Some(c) if !matches!(c, Value::Nil) => Shader::SolidColor(color::parse(&c)?),
            _ => Shader::SolidColor(Color::BLACK),
        },
    };
    let mut paint = Paint {
        shader,
        anti_alias: opt_bool(&m, "anti-alias").unwrap_or(true),
        ..Paint::default()
    };
    if let Some(name) = opt_name(&m, "blend")? {
        paint.blend_mode = blend_mode(&name)?;
    }

    let rule = match opt_name(&m, "fill-rule")? {
        Some(name) => fill_rule(&name)?,
        None => FillRule::Winding,
    };
    let transform = match opt(&m, "transform") {
        Some(t) => parse_transform(&t)?,
        None => Transform::identity(),
    };

    Ok(DrawOpts {
        paint,
        fill_rule: rule,
        transform,
    })
}

/// Parse the stroke-specific options out of the same map.
pub fn parse_stroke(opts: &Value) -> Result<Stroke, String> {
    let m = opts_map(opts)?;
    let mut stroke = Stroke {
        width: opt_f32(&m, "width")?.unwrap_or(1.0),
        miter_limit: opt_f32(&m, "miter-limit")?.unwrap_or(4.0),
        ..Stroke::default()
    };
    if let Some(name) = opt_name(&m, "line-cap")? {
        stroke.line_cap = line_cap(&name)?;
    }
    if let Some(name) = opt_name(&m, "line-join")? {
        stroke.line_join = line_join(&name)?;
    }
    if let Some(d) = opt(&m, "dash")
        && !matches!(d, Value::Nil)
    {
        // Either `[6 3]` or `{:array [6 3] :offset 2}`.
        let (array, offset) = match &d {
            Value::Vector(_) => (as_f32_vec(&d, ":dash")?, 0.0),
            Value::Map(_) => {
                let dm = opts_map(&d)?;
                let arr = opt(&dm, "array")
                    .ok_or_else(|| ":dash map needs an :array".to_string())
                    .and_then(|a| as_f32_vec(&a, ":dash :array"))?;
                (arr, opt_f32(&dm, "offset")?.unwrap_or(0.0))
            }
            other => {
                return Err(format!(
                    ":dash must be a vector or a map, got {}",
                    other.type_name()
                ));
            }
        };
        stroke.dash = StrokeDash::new(array, offset);
        if stroke.dash.is_none() {
            return Err(":dash needs an even, non-empty array of positive lengths".to_string());
        }
    }
    Ok(stroke)
}
