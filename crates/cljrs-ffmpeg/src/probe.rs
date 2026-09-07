//! `ffprobe` → Clojure data.
//!
//! `probe` runs `ffprobe -print_format json -show_format -show_streams` and
//! converts the result into ordinary Clojure maps, vectors, strings and
//! numbers, so a caller uses `get-in` and destructuring rather than a bespoke
//! accessor API.
//!
//! Two conversions are worth knowing about:
//!
//! - **Keys become keywords with `_` rewritten to `-`**, so JSON `codec_name`
//!   reads as `:codec-name`. The names are FFmpeg's, only re-spelled.
//! - **Values are left exactly as ffprobe emits them.** ffprobe reports many
//!   numbers as JSON *strings* (`"duration": "12.345"`, `"nb_frames": "300"`),
//!   and this crate does not guess which ones to coerce — silently turning a
//!   string into a number is the kind of helpfulness that hides a schema change.
//!   Use `duration` and `dimensions` for the two that matter in practice; parse
//!   the rest yourself.

use cljrs_gc::GcPtr;
use cljrs_value::{Keyword, MapValue, PersistentVector, Value};
use serde_json::Value as Json;

use crate::proc;

/// Convert a JSON document to Clojure data.
pub fn json_to_value(j: &Json) -> Value {
    match j {
        Json::Null => Value::Nil,
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Long(i)
            } else {
                Value::Double(n.as_f64().unwrap_or(f64::NAN))
            }
        }
        Json::String(s) => Value::string(s.clone()),
        Json::Array(items) => Value::Vector(GcPtr::new(PersistentVector::from_iter(
            items.iter().map(json_to_value),
        ))),
        Json::Object(entries) => Value::Map(MapValue::from_pairs(
            entries
                .iter()
                .map(|(k, v)| {
                    (
                        Value::keyword(Keyword::simple(k.replace('_', "-").as_str())),
                        json_to_value(v),
                    )
                })
                .collect(),
        )),
    }
}

fn probe_json(path: &str) -> Result<Json, String> {
    let args: Vec<String> = [
        "-v",
        "error",
        "-print_format",
        "json",
        "-show_format",
        "-show_streams",
        path,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let out = proc::run_checked(&proc::ffprobe_bin(), &args)?;
    serde_json::from_str(&out).map_err(|e| format!("ffprobe returned invalid JSON: {e}"))
}

/// `{:format {...} :streams [...]}` for a media file.
pub fn probe(path: &str) -> Result<Value, String> {
    Ok(json_to_value(&probe_json(path)?))
}

/// Duration in seconds, read from the container.
pub fn duration(path: &str) -> Result<f64, String> {
    let args: Vec<String> = [
        "-v",
        "error",
        "-show_entries",
        "format=duration",
        "-of",
        "default=noprint_wrappers=1:nokey=1",
        path,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let out = proc::run_checked(&proc::ffprobe_bin(), &args)?;
    out.trim()
        .parse::<f64>()
        .map_err(|_| format!("{path}: no duration reported (got {:?})", out.trim()))
}

/// `[width height]` of the first video stream.
pub fn dimensions(path: &str) -> Result<Vec<Value>, String> {
    let args: Vec<String> = [
        "-v",
        "error",
        "-select_streams",
        "v:0",
        "-show_entries",
        "stream=width,height",
        "-of",
        "csv=p=0",
        path,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let out = proc::run_checked(&proc::ffprobe_bin(), &args)?;
    let line = out
        .lines()
        .next()
        .ok_or_else(|| format!("{path}: no video stream"))?;
    let mut parts = line.trim().split(',');
    let w = parts
        .next()
        .and_then(|s| s.trim().parse::<i64>().ok())
        .ok_or_else(|| format!("{path}: no video stream width"))?;
    let h = parts
        .next()
        .and_then(|s| s.trim().parse::<i64>().ok())
        .ok_or_else(|| format!("{path}: no video stream height"))?;
    Ok(vec![Value::Long(w), Value::Long(h)])
}
