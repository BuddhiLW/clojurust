//! Colour specs: how Clojure data becomes a `tiny_skia::Color`.
//!
//! Four spellings are accepted, in this order of preference when writing
//! Clojure:
//!
//! | Spelling | Example | Range |
//! |---|---|---|
//! | keyword | `:crimson` | one of the named colours below |
//! | hex string | `"#ff8800"`, `"#f80"`, `"#ff8800cc"` | 3, 4, 6 or 8 digits |
//! | integer vector | `[255 136 0]`, `[255 136 0 200]` | each channel `0–255` |
//! | float vector | `[1.0 0.53 0.0]`, `[1.0 0.53 0.0 0.8]` | each channel `0.0–1.0` |
//!
//! A vector is read as integer channels when **every** element is a `Long`,
//! and as float channels as soon as one element is a `Double`. `[1 1 1]` is
//! therefore near-black, while `[1.0 1.0 1.0]` is white — the same rule
//! Clojure itself applies to `1` vs `1.0`.

use cljrs_value::Value;
use tiny_skia::Color;

use crate::args::as_f32;

/// The named colours, resolvable as `:name` or `"name"`.
///
/// The CSS basic-16 plus the handful of extras that come up constantly in
/// charts and diagrams.
const NAMED: &[(&str, u32)] = &[
    ("transparent", 0x00000000),
    ("black", 0xff000000),
    ("white", 0xffffffff),
    ("red", 0xffff0000),
    ("lime", 0xff00ff00),
    ("green", 0xff008000),
    ("blue", 0xff0000ff),
    ("yellow", 0xffffff00),
    ("cyan", 0xff00ffff),
    ("aqua", 0xff00ffff),
    ("magenta", 0xffff00ff),
    ("fuchsia", 0xffff00ff),
    ("silver", 0xffc0c0c0),
    ("gray", 0xff808080),
    ("grey", 0xff808080),
    ("maroon", 0xff800000),
    ("olive", 0xff808000),
    ("teal", 0xff008080),
    ("navy", 0xff000080),
    ("purple", 0xff800080),
    ("orange", 0xffffa500),
    ("crimson", 0xffdc143c),
    ("gold", 0xffffd700),
    ("indigo", 0xff4b0082),
    ("violet", 0xffee82ee),
    ("pink", 0xffffc0cb),
    ("brown", 0xffa52a2a),
    ("beige", 0xfff5f5dc),
    ("ivory", 0xfffffff0),
    ("salmon", 0xfffa8072),
    ("khaki", 0xfff0e68c),
    ("turquoise", 0xff40e0d0),
    ("steelblue", 0xff4682b4),
    ("slategray", 0xff708090),
    ("slategrey", 0xff708090),
];

fn from_argb(argb: u32) -> Color {
    Color::from_rgba8(
        ((argb >> 16) & 0xff) as u8,
        ((argb >> 8) & 0xff) as u8,
        (argb & 0xff) as u8,
        ((argb >> 24) & 0xff) as u8,
    )
}

fn named(name: &str) -> Option<Color> {
    let lower = name.to_ascii_lowercase();
    NAMED
        .iter()
        .find(|(n, _)| *n == lower)
        .map(|(_, argb)| from_argb(*argb))
}

/// Parse `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`.
fn hex(s: &str) -> Result<Color, String> {
    let digits = s.trim_start_matches('#');
    let nibble = |i: usize| -> Result<u8, String> {
        u8::from_str_radix(&digits[i..i + 1], 16)
            .map_err(|_| format!("not a hex colour: \"{s}\""))
            .map(|n| n * 17) // 0xf → 0xff
    };
    let byte = |i: usize| -> Result<u8, String> {
        u8::from_str_radix(&digits[i..i + 2], 16).map_err(|_| format!("not a hex colour: \"{s}\""))
    };
    match digits.len() {
        3 => Ok(Color::from_rgba8(nibble(0)?, nibble(1)?, nibble(2)?, 255)),
        4 => Ok(Color::from_rgba8(
            nibble(0)?,
            nibble(1)?,
            nibble(2)?,
            nibble(3)?,
        )),
        6 => Ok(Color::from_rgba8(byte(0)?, byte(2)?, byte(4)?, 255)),
        8 => Ok(Color::from_rgba8(byte(0)?, byte(2)?, byte(4)?, byte(6)?)),
        n => Err(format!(
            "a hex colour needs 3, 4, 6 or 8 digits, \"{s}\" has {n}"
        )),
    }
}

/// Parse any accepted colour spelling.
pub fn parse(v: &Value) -> Result<Color, String> {
    match v {
        Value::Keyword(k) => {
            let name = k.get().name.to_string();
            named(&name).ok_or_else(|| format!("unknown colour :{name}"))
        }
        Value::Str(s) => {
            let s = s.get();
            if s.starts_with('#') {
                hex(s)
            } else {
                named(s).ok_or_else(|| format!("unknown colour \"{s}\""))
            }
        }
        Value::Vector(vec) => {
            let vec = vec.get();
            if !(3..=4).contains(&vec.count()) {
                return Err(format!(
                    "a colour vector needs 3 or 4 channels, got {}",
                    vec.count()
                ));
            }
            // All-Long means 0–255 channels; any Double means 0.0–1.0.
            let integral = vec.iter().all(|e| matches!(e, Value::Long(_)));
            let mut ch = [0.0f32; 4];
            ch[3] = if integral { 255.0 } else { 1.0 };
            for (i, e) in vec.iter().enumerate() {
                ch[i] = as_f32(e)?;
            }
            let c = if integral {
                Color::from_rgba8(
                    clamp_u8(ch[0]),
                    clamp_u8(ch[1]),
                    clamp_u8(ch[2]),
                    clamp_u8(ch[3]),
                )
            } else {
                Color::from_rgba(
                    ch[0].clamp(0.0, 1.0),
                    ch[1].clamp(0.0, 1.0),
                    ch[2].clamp(0.0, 1.0),
                    ch[3].clamp(0.0, 1.0),
                )
                .ok_or_else(|| "colour channels must be finite".to_string())?
            };
            Ok(c)
        }
        other => Err(format!(
            "expected a colour (keyword, \"#hex\" or [r g b a]), got {}",
            other.type_name()
        )),
    }
}

fn clamp_u8(f: f32) -> u8 {
    f.clamp(0.0, 255.0).round() as u8
}

/// Render a colour back as a `[r g b a]` vector of 0–255 integers — the shape
/// `pixel` returns, so `(set-pixel! c x y (pixel c x y))` round-trips.
pub fn to_rgba_vec(c: tiny_skia::ColorU8) -> Vec<Value> {
    vec![
        Value::Long(c.red() as i64),
        Value::Long(c.green() as i64),
        Value::Long(c.blue() as i64),
        Value::Long(c.alpha() as i64),
    ]
}
