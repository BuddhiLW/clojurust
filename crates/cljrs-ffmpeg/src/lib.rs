//! FFmpeg media probing, transcoding and frame streaming for Clojurust.
//!
//! Registers the `cljrs.ffmpeg` namespace. Three things live here:
//!
//! | Area | Functions |
//! |---|---|
//! | inspect | `available?`, `version`, `probe`, `duration`, `dimensions` |
//! | one-shot | `run`, `extract-frame!`, `transcode!` |
//! | stream | `video-writer`, `write-frame!`, `close-writer!`, `writer-closed?`, `frames-written` |
//!
//! ```clojure
//! (require '[cljrs.ffmpeg :as ff] '[cljrs.raster :as r])
//!
//! (ff/probe "clip.mp4")                       ;=> {:format {...} :streams [...]}
//! (ff/extract-frame! "clip.mp4" "/tmp/f.png" {:time 3.5})
//!
//! (let [w (ff/video-writer "/tmp/out.mp4" {:width 320 :height 180 :fps 24})]
//!   (dotimes [_ 48] (ff/write-frame! w (doto (r/canvas 320 180) (r/clear! :black))))
//!   (ff/close-writer! w))
//! ```
//!
//! A frame is straight-RGBA bytes, which is exactly what
//! `cljrs.raster/rgba-bytes` returns:
//!
//! ```clojure
//! (ff/write-frame! w (r/rgba-bytes canvas))
//! ```
//!
//! The two crates compose **in Clojure, not in Cargo**. Neither depends on the
//! other: an encoder that pulls in a rasterizer is a layering inversion, and
//! the real contract between them is that byte layout. Passing a canvas
//! straight to `write-frame!` therefore raises an error naming the conversion
//! rather than failing obscurely.
//!
//! Binaries are found on `PATH`, overridable with `$CLJRS_FFMPEG` and
//! `$CLJRS_FFPROBE`. See [`proc`] for why this drives the executables rather
//! than linking `libav*`.

use std::sync::Arc;

use cljrs_interop::{Registry, Value, wrap_fn_variadic, wrap_fn0, wrap_fn1, wrap_fn2};
use cljrs_runtime::env::env::GlobalEnv;
use cljrs_value::{Keyword, MapValue, Resource, ResourceHandle};

pub mod probe;
pub mod proc;
pub mod writer;

pub use writer::{Outcome, VideoWriter, WriterSpec};

/// The Clojure namespace this crate registers.
pub const NS: &str = "cljrs.ffmpeg";

fn kw(name: &str) -> Value {
    Value::keyword(Keyword::simple(name))
}

fn opts_map(v: &Value) -> Result<MapValue, String> {
    match v {
        Value::Nil => Ok(MapValue::empty()),
        Value::Map(m) => Ok(m.clone()),
        other => Err(format!(
            "expected an options map, got {}",
            other.type_name()
        )),
    }
}

fn opt(m: &MapValue, key: &str) -> Option<Value> {
    match m.get(&kw(key)) {
        Some(Value::Nil) | None => None,
        Some(v) => Some(v),
    }
}

fn opt_str(m: &MapValue, key: &str) -> Result<Option<String>, String> {
    match opt(m, key) {
        None => Ok(None),
        Some(Value::Str(s)) => Ok(Some(s.get().clone())),
        Some(Value::Keyword(k)) => Ok(Some(k.get().name.to_string())),
        Some(other) => Err(format!(
            ":{key} must be a string or keyword, got {}",
            other.type_name()
        )),
    }
}

fn opt_f64(m: &MapValue, key: &str) -> Result<Option<f64>, String> {
    match opt(m, key) {
        None => Ok(None),
        Some(Value::Long(n)) => Ok(Some(n as f64)),
        Some(Value::Double(d)) => Ok(Some(d)),
        Some(other) => Err(format!(
            ":{key} must be a number, got {}",
            other.type_name()
        )),
    }
}

fn opt_i64(m: &MapValue, key: &str) -> Result<Option<i64>, String> {
    match opt(m, key) {
        None => Ok(None),
        Some(Value::Long(n)) => Ok(Some(n)),
        Some(Value::Double(d)) => Ok(Some(d as i64)),
        Some(other) => Err(format!(
            ":{key} must be an integer, got {}",
            other.type_name()
        )),
    }
}

fn opt_u32(m: &MapValue, key: &str) -> Result<Option<u32>, String> {
    Ok(match opt_i64(m, key)? {
        None => None,
        Some(n) => Some(
            u32::try_from(n)
                .map_err(|_| format!(":{key} must be a non-negative integer, got {n}"))?,
        ),
    })
}

/// Read a vector of strings, the shape every raw-argument option takes.
fn as_strings(v: &Value, what: &str) -> Result<Vec<String>, String> {
    match v {
        Value::Nil => Ok(Vec::new()),
        Value::Vector(vec) => vec
            .get()
            .iter()
            .map(|e| match e {
                Value::Str(s) => Ok(s.get().clone()),
                Value::Keyword(k) => Ok(k.get().name.to_string()),
                Value::Long(n) => Ok(n.to_string()),
                Value::Double(d) => Ok(d.to_string()),
                other => Err(format!(
                    "{what} must hold strings or numbers, got {}",
                    other.type_name()
                )),
            })
            .collect(),
        other => Err(format!(
            "{what} must be a vector, got {}",
            other.type_name()
        )),
    }
}

/// A `-vf scale=w:h` filter from a `:scale [w h]` option.
fn scale_filter(m: &MapValue) -> Result<Option<String>, String> {
    match opt(m, "scale") {
        None => Ok(None),
        Some(Value::Vector(vec)) if vec.get().count() == 2 => {
            let v = vec.get();
            let num = |i: usize| -> Result<String, String> {
                match v.nth(i).expect("count checked") {
                    Value::Long(n) => Ok(n.to_string()),
                    // -1 preserves the aspect ratio; ffmpeg spells it that way.
                    Value::Double(d) => Ok((*d as i64).to_string()),
                    other => Err(format!(
                        ":scale must be [width height] integers, got {}",
                        other.type_name()
                    )),
                }
            };
            Ok(Some(format!("scale={}:{}", num(0)?, num(1)?)))
        }
        Some(other) => Err(format!(
            ":scale must be a [width height] vector, got {}",
            other.type_name()
        )),
    }
}

/// Run `f` against the `VideoWriter` behind a resource value.
///
/// `ResourceHandle::downcast` hands back a borrow tied to the handle, so the
/// writer is reached through a closure rather than cloned out — an `Arc<dyn
/// Resource>` cannot be re-pointed at the concrete type without unsafe, and
/// nothing here needs it to outlive the call.
fn with_writer<R>(v: &Value, f: impl FnOnce(&VideoWriter) -> R) -> Result<R, String> {
    match v {
        Value::Resource(handle) => match handle.downcast::<VideoWriter>() {
            Some(w) => Ok(f(w)),
            None => Err(format!(
                "expected a video-writer, got a {} resource",
                handle.resource_type()
            )),
        },
        other => Err(format!(
            "expected a video-writer, got {}",
            other.type_name()
        )),
    }
}

// ── Registration ──────────────────────────────────────────────────────────────

/// Register the `cljrs.ffmpeg` namespace into `globals`. Idempotent.
pub fn init(globals: &Arc<GlobalEnv>) {
    if globals.is_loaded(NS) {
        return;
    }
    globals.get_or_create_ns(NS);
    globals.refer_core(NS);
    let mut registry = Registry::for_require(globals.clone());
    register(&mut registry);
}

/// Register every `cljrs.ffmpeg` function through an existing `Registry`.
pub fn register(registry: &mut Registry) {
    register_inspect(registry);
    register_oneshot(registry);
    register_stream(registry);
    // Native namespaces have no source file; without this `require` searches
    // the source path and fails.
    registry.env().mark_loaded(NS);
}

fn register_inspect(registry: &mut Registry) {
    registry.define(
        "cljrs.ffmpeg/available?",
        wrap_fn0("cljrs.ffmpeg/available?", || -> Result<bool, String> {
            Ok(proc::available(&proc::ffmpeg_bin()) && proc::available(&proc::ffprobe_bin()))
        }),
    );

    registry.define(
        "cljrs.ffmpeg/version",
        wrap_fn0("cljrs.ffmpeg/version", || -> Result<String, String> {
            let out = proc::run_checked(&proc::ffmpeg_bin(), &["-version".to_string()])?;
            Ok(out.lines().next().unwrap_or("").trim().to_string())
        }),
    );

    registry.define(
        "cljrs.ffmpeg/probe",
        wrap_fn1("cljrs.ffmpeg/probe", |p: String| -> Result<Value, String> {
            probe::probe(&p)
        }),
    );

    registry.define(
        "cljrs.ffmpeg/duration",
        wrap_fn1(
            "cljrs.ffmpeg/duration",
            |p: String| -> Result<f64, String> { probe::duration(&p) },
        ),
    );

    registry.define(
        "cljrs.ffmpeg/dimensions",
        wrap_fn1(
            "cljrs.ffmpeg/dimensions",
            |p: String| -> Result<Vec<Value>, String> { probe::dimensions(&p) },
        ),
    );
}

fn register_oneshot(registry: &mut Registry) {
    // (run ["-i" "in.mp4" ... "out.mp4"]) => {:exit n :out "…" :error "…"}
    //
    // The escape hatch: anything the typed wrappers do not cover is still one
    // vector of arguments away, with no shell in between.
    registry.define(
        "cljrs.ffmpeg/run",
        wrap_fn1("cljrs.ffmpeg/run", |args: Value| -> Result<Value, String> {
            let args = as_strings(&args, "ffmpeg arguments")?;
            let out = proc::run(&proc::ffmpeg_bin(), &args)?;
            Ok(Value::Map(MapValue::from_pairs(vec![
                (
                    kw("exit"),
                    Value::Long(out.status.code().unwrap_or(-1) as i64),
                ),
                (
                    kw("out"),
                    Value::string(String::from_utf8_lossy(&out.stdout).to_string()),
                ),
                (
                    kw("error"),
                    Value::string(String::from_utf8_lossy(&out.stderr).to_string()),
                ),
            ])))
        }),
    );

    // (extract-frame! "in.mp4" "out.png" {:time 3.5 :scale [640 -1]})
    registry.define(
        "cljrs.ffmpeg/extract-frame!",
        wrap_fn_variadic(
            "cljrs.ffmpeg/extract-frame!",
            2,
            |a: &[Value]| -> Result<Value, String> {
                let input = string_arg(&a[0], "input path")?;
                let output = string_arg(&a[1], "output path")?;
                let m = opts_map(a.get(2).unwrap_or(&Value::Nil))?;

                let mut args: Vec<String> = vec!["-y".into(), "-hide_banner".into()];
                args.extend(["-loglevel".into(), "error".into()]);
                // `-ss` before `-i` seeks by keyframe index instead of decoding
                // the whole prefix — the difference between milliseconds and
                // minutes on a long file.
                if let Some(t) = opt_f64(&m, "time")? {
                    args.extend(["-ss".into(), format!("{t}")]);
                }
                args.extend(["-i".into(), input]);
                args.extend(["-frames:v".into(), "1".into()]);
                if let Some(f) = scale_filter(&m)? {
                    args.extend(["-vf".into(), f]);
                }
                args.extend(as_strings(&opt(&m, "args").unwrap_or(Value::Nil), ":args")?);
                args.push(output.clone());

                proc::run_checked(&proc::ffmpeg_bin(), &args)?;
                Ok(Value::string(output))
            },
        ),
    );

    // (transcode! "in.mov" "out.mp4" {:codec "libx264" :crf 20 :scale [1280 -1]})
    registry.define(
        "cljrs.ffmpeg/transcode!",
        wrap_fn_variadic(
            "cljrs.ffmpeg/transcode!",
            2,
            |a: &[Value]| -> Result<Value, String> {
                let input = string_arg(&a[0], "input path")?;
                let output = string_arg(&a[1], "output path")?;
                let m = opts_map(a.get(2).unwrap_or(&Value::Nil))?;

                let mut args: Vec<String> = Vec::new();
                args.push(if opt(&m, "overwrite").is_some_and(is_false) {
                    "-n".into()
                } else {
                    "-y".into()
                });
                args.extend([
                    "-hide_banner".into(),
                    "-loglevel".into(),
                    opt_str(&m, "loglevel")?.unwrap_or_else(|| "error".to_string()),
                ]);
                if let Some(t) = opt_f64(&m, "start")? {
                    args.extend(["-ss".into(), format!("{t}")]);
                }
                args.extend(["-i".into(), input]);
                if let Some(d) = opt_f64(&m, "duration")? {
                    args.extend(["-t".into(), format!("{d}")]);
                }
                if let Some(c) = opt_str(&m, "codec")? {
                    args.extend(["-c:v".into(), c]);
                }
                if let Some(c) = opt_str(&m, "audio-codec")? {
                    args.extend(["-c:a".into(), c]);
                }
                if let Some(crf) = opt_i64(&m, "crf")? {
                    args.extend(["-crf".into(), crf.to_string()]);
                }
                if let Some(p) = opt_str(&m, "preset")? {
                    args.extend(["-preset".into(), p]);
                }
                if let Some(fps) = opt_f64(&m, "fps")? {
                    args.extend(["-r".into(), format!("{fps}")]);
                }
                if let Some(f) = scale_filter(&m)? {
                    args.extend(["-vf".into(), f]);
                }
                if let Some(pf) = opt_str(&m, "pix-fmt")? {
                    args.extend(["-pix_fmt".into(), pf]);
                }
                args.extend(as_strings(&opt(&m, "args").unwrap_or(Value::Nil), ":args")?);
                args.push(output.clone());

                proc::run_checked(&proc::ffmpeg_bin(), &args)?;
                Ok(Value::string(output))
            },
        ),
    );
}

fn register_stream(registry: &mut Registry) {
    // (video-writer "out.mp4" {:width 640 :height 360 :fps 30})
    registry.define(
        "cljrs.ffmpeg/video-writer",
        wrap_fn_variadic(
            "cljrs.ffmpeg/video-writer",
            1,
            |a: &[Value]| -> Result<Value, String> {
                let path = string_arg(&a[0], "output path")?;
                let m = opts_map(a.get(1).unwrap_or(&Value::Nil))?;
                let spec = WriterSpec {
                    width: opt_u32(&m, "width")?
                        .ok_or_else(|| "a video writer needs :width".to_string())?,
                    height: opt_u32(&m, "height")?
                        .ok_or_else(|| "a video writer needs :height".to_string())?,
                    fps: opt_f64(&m, "fps")?.unwrap_or(30.0),
                    codec: opt_str(&m, "codec")?.unwrap_or_else(|| "libx264".to_string()),
                    pix_fmt: opt_str(&m, "pix-fmt")?.unwrap_or_else(|| "yuv420p".to_string()),
                    crf: opt_i64(&m, "crf")?,
                    preset: opt_str(&m, "preset")?,
                    loglevel: opt_str(&m, "loglevel")?.unwrap_or_else(|| "error".to_string()),
                    overwrite: !opt(&m, "overwrite").is_some_and(is_false),
                    extra: as_strings(&opt(&m, "args").unwrap_or(Value::Nil), ":args")?,
                };
                let w = VideoWriter::spawn(&path, &spec)?;
                Ok(Value::Resource(ResourceHandle::new(w)))
            },
        ),
    );

    registry.define(
        "cljrs.ffmpeg/write-frame!",
        wrap_fn2(
            "cljrs.ffmpeg/write-frame!",
            |w: Value, frame: Value| -> Result<Value, String> {
                let bytes = frame_bytes(&frame)?;
                with_writer(&w, |writer| writer.write_frame(&bytes))??;
                Ok(w.clone())
            },
        ),
    );

    registry.define(
        "cljrs.ffmpeg/close-writer!",
        wrap_fn1(
            "cljrs.ffmpeg/close-writer!",
            |w: Value| -> Result<Value, String> {
                with_writer(&w, |writer| {
                    writer
                        .finish()
                        .map(|o| writer::outcome_value(&writer.path, &o))
                })?
            },
        ),
    );

    registry.define(
        "cljrs.ffmpeg/writer-closed?",
        wrap_fn1(
            "cljrs.ffmpeg/writer-closed?",
            |w: Value| -> Result<bool, String> { with_writer(&w, |writer| writer.is_closed()) },
        ),
    );

    registry.define(
        "cljrs.ffmpeg/frames-written",
        wrap_fn1(
            "cljrs.ffmpeg/frames-written",
            |w: Value| -> Result<i64, String> {
                with_writer(&w, |writer| writer.frames_written() as i64)
            },
        ),
    );

    registry.define(
        "cljrs.ffmpeg/writer?",
        wrap_fn1("cljrs.ffmpeg/writer?", |v: Value| -> Result<bool, String> {
            Ok(with_writer(&v, |_| ()).is_ok())
        }),
    );
}

fn is_false(v: Value) -> bool {
    matches!(v, Value::Bool(false))
}

fn string_arg(v: &Value, what: &str) -> Result<String, String> {
    match v {
        Value::Str(s) => Ok(s.get().clone()),
        other => Err(format!(
            "{what} must be a string, got {}",
            other.type_name()
        )),
    }
}

/// A frame is straight-RGBA bytes: a `ByteArray`, a `ByteBlob`, or a vector of
/// integers in 0-255.
fn frame_bytes(v: &Value) -> Result<Vec<u8>, String> {
    match v {
        // A canvas is the obvious thing to reach for, and this crate cannot
        // accept one (see the module docs), so name the conversion instead of
        // reporting an unhelpful type error.
        Value::NativeObject(obj) if obj.get().type_tag() == "Canvas" => Err(
            "a frame must be bytes, not a canvas — pass (cljrs.raster/rgba-bytes c)".to_string(),
        ),
        Value::ByteBlob(b) => Ok(b.to_vec()),
        Value::ByteArray(a) => {
            let guard = a
                .get()
                .lock()
                .map_err(|_| "byte array lock poisoned".to_string())?;
            Ok(guard.iter().map(|b| *b as u8).collect())
        }
        Value::Vector(vec) => vec
            .get()
            .iter()
            .map(|e| match e {
                Value::Long(n) if (0..=255).contains(n) => Ok(*n as u8),
                Value::Long(n) => Err(format!("byte out of range 0–255: {n}")),
                other => Err(format!(
                    "a frame byte vector holds integers 0–255, got {}",
                    other.type_name()
                )),
            })
            .collect(),
        other => Err(format!(
            "a frame must be a canvas or straight-RGBA bytes, got {}",
            other.type_name()
        )),
    }
}
