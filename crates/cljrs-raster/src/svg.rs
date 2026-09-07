//! SVG rasterization, via [`resvg`].
//!
//! This is also how a canvas gets **text**. `tiny_skia` is a Skia subset with
//! no font stack, so there is no `draw-text!` in this crate; `resvg` brings a
//! parser, a shaper and a font database, and an `<svg>` document with a
//! `<text>` element in it is the supported way to put glyphs on a canvas.
//!
//! ```clojure
//! (r/render-svg "<svg xmlns='http://www.w3.org/2000/svg' width='200' height='60'>
//!                  <text x='10' y='40' font-size='28' fill='#4682b4'>hello</text>
//!                </svg>")
//! ```
//!
//! It also makes any SVG-producing tool a frame source: render each frame to
//! SVG, rasterize it here, and pipe the canvases to `cljrs.ffmpeg`.

use std::sync::{Arc, OnceLock};

use cljrs_value::Value;
use resvg::usvg;
use tiny_skia::{Pixmap, Transform};

use crate::args::{opt_f32, opt_name, opts_map};
use crate::canvas::{Canvas, to_bytes};
use crate::color;

/// Families to prefer as the default when the document names none. Ordered by
/// how likely they are to be present and to carry wide coverage.
const PREFERRED: &[&str] = &[
    "DejaVu Sans",
    "Liberation Sans",
    "Noto Sans",
    "Arial",
    "Helvetica",
    "Segoe UI",
    "Cantarell",
    "Ubuntu",
    "FreeSans",
];

/// The system font database and a default family that actually exists in it,
/// scanned once per process.
///
/// `load_system_fonts` walks the platform's font directories and costs real
/// time; a document that renders 900 frames should pay for that once, not 900
/// times. `usvg::Options` holds its database as an `Arc`, so every render
/// shares this one rather than cloning it.
static FONTS: OnceLock<(Arc<usvg::fontdb::Database>, String)> = OnceLock::new();

/// Pick a default family present in `db`.
///
/// usvg's own default is "Times New Roman", which is absent from a stock Linux
/// install, so every `<text>` element renders as nothing and the only clue is
/// a log line. Choosing from what is actually installed makes text work out of
/// the box, and naming a family in the SVG still overrides it.
fn default_family(db: &usvg::fontdb::Database) -> String {
    let installed: Vec<&str> = db
        .faces()
        .flat_map(|f| f.families.iter().map(|(name, _)| name.as_str()))
        .collect();
    for want in PREFERRED {
        if installed.iter().any(|have| have.eq_ignore_ascii_case(want)) {
            return (*want).to_string();
        }
    }
    installed
        .first()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "sans-serif".to_string())
}

fn fonts() -> (Arc<usvg::fontdb::Database>, String) {
    let (db, family) = FONTS.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        let family = default_family(&db);
        // Generic names in the document resolve to the same choice, so
        // `font-family="sans-serif"` is not a second way to get nothing.
        db.set_sans_serif_family(family.clone());
        db.set_serif_family(family.clone());
        db.set_monospace_family(family.clone());
        (Arc::new(db), family)
    });
    (db.clone(), family.clone())
}

/// Accept the SVG document as a string or as bytes.
fn document(v: &Value) -> Result<Vec<u8>, String> {
    match v {
        Value::Str(s) => Ok(s.get().as_bytes().to_vec()),
        other => to_bytes(other).map_err(|_| {
            format!(
                "expected an SVG document as a string or bytes, got {}",
                other.type_name()
            )
        }),
    }
}

/// Parse `svg` into a usvg tree under the options in `opts`.
fn parse(svg: &[u8], opts: &Value) -> Result<usvg::Tree, String> {
    let m = opts_map(opts)?;
    let mut options = usvg::Options {
        dpi: opt_f32(&m, "dpi")?.unwrap_or(96.0),
        font_size: opt_f32(&m, "font-size")?.unwrap_or(12.0),
        ..usvg::Options::default()
    };
    if let Some(dir) = opt_name(&m, "resources-dir")? {
        options.resources_dir = Some(std::path::PathBuf::from(dir));
    }
    let (db, default) = fonts();
    options.fontdb = db;
    options.font_family = opt_name(&m, "font-family")?.unwrap_or(default);

    usvg::Tree::from_data(svg, &options).map_err(|e| format!("invalid SVG: {e}"))
}

/// The output size, and the transform that maps the document onto it.
///
/// `:width`/`:height` give an explicit size; supplying only one scales the
/// other to keep the aspect ratio, which is what almost every caller means by
/// "render this 1920 wide". `:scale` multiplies the document's own size.
fn target(tree: &usvg::Tree, opts: &Value) -> Result<(u32, u32, Transform), String> {
    let m = opts_map(opts)?;
    let size = tree.size();
    let (iw, ih) = (size.width(), size.height());
    if iw <= 0.0 || ih <= 0.0 {
        return Err(format!("SVG has a degenerate size ({iw}x{ih})"));
    }

    let scale = opt_f32(&m, "scale")?;
    let w = opt_f32(&m, "width")?;
    let h = opt_f32(&m, "height")?;

    let (tw, th) = match (w, h, scale) {
        (Some(w), Some(h), _) => (w, h),
        (Some(w), None, _) => (w, ih * (w / iw)),
        (None, Some(h), _) => (iw * (h / ih), h),
        (None, None, Some(s)) => (iw * s, ih * s),
        (None, None, None) => (iw, ih),
    };
    if tw < 1.0 || th < 1.0 {
        return Err(format!("SVG target size rounds to nothing ({tw}x{th})"));
    }

    Ok((
        tw.round() as u32,
        th.round() as u32,
        Transform::from_scale(tw / iw, th / ih),
    ))
}

/// Rasterize an SVG document to a new canvas.
pub fn render(svg: &Value, opts: &Value) -> Result<Value, String> {
    let data = document(svg)?;
    let tree = parse(&data, opts)?;
    let (w, h, transform) = target(&tree, opts)?;

    let mut pixmap =
        Pixmap::new(w, h).ok_or_else(|| format!("cannot allocate a {w}x{h} canvas"))?;

    // Transparent unless asked otherwise: an SVG rarely paints its own
    // background, and compositing onto transparency is the composable default.
    let m = opts_map(opts)?;
    if let Some(bg) = crate::args::opt(&m, "background")
        && !matches!(bg, Value::Nil)
    {
        pixmap.fill(color::parse(&bg)?);
    }

    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Ok(Canvas::wrap(pixmap))
}

/// Rasterize an SVG file.
///
/// `:resources-dir` defaults to the file's own directory, so relative `href`s
/// inside the document resolve the way they do when a browser opens it.
pub fn render_file(path: &str, opts: &Value) -> Result<Value, String> {
    let data = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let m = opts_map(opts)?;
    let opts = if crate::args::opt(&m, "resources-dir").is_some() {
        opts.clone()
    } else {
        let dir = std::path::Path::new(path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        Value::Map(m.assoc(crate::args::kw("resources-dir"), Value::string(dir)))
    };
    let doc = Value::string(String::from_utf8_lossy(&data).to_string());
    render(&doc, &opts)
}

/// The document's own `[width height]`, before any scaling.
pub fn size(svg: &Value, opts: &Value) -> Result<Vec<Value>, String> {
    let data = document(svg)?;
    let tree = parse(&data, opts)?;
    let s = tree.size();
    Ok(vec![
        Value::Double(s.width() as f64),
        Value::Double(s.height() as f64),
    ])
}
