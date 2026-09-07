//! A long-lived encoder you push frames into.
//!
//! `video-writer` spawns one `ffmpeg` child reading **raw RGBA from stdin** and
//! keeps it alive; `write-frame!` writes one frame's bytes; `close-writer!`
//! closes the pipe, waits for the encoder to flush, and reports how it went.
//! That makes rendering an animation the obvious loop:
//!
//! ```clojure
//! (let [w (ff/video-writer "/tmp/spin.mp4" {:width 640 :height 360 :fps 30})]
//!   (doseq [i (range 90)]
//!     (let [c (r/canvas 640 360)]
//!       (r/clear! c :white)
//!       (r/fill-circle! c (+ 320 (* 200 (Math/cos (/ i 14.0)))) 180 40 {:color :crimson})
//!       (ff/write-frame! w c)))
//!   (ff/close-writer! w))
//! ```
//!
//! A writer is a [`Resource`], not a `NativeObject`: it owns an OS process and
//! a pipe, and the GC has no finalizers, so it needs the `Arc`-refcounted,
//! explicitly-closed lifecycle that the rest of Clojurust's I/O uses. Dropping
//! the last reference without closing leaves the child to be reaped by the OS —
//! call `close-writer!`.
//!
//! ## Frame size is checked, not trusted
//!
//! `-f rawvideo` has no framing: ffmpeg slices stdin into `width*height*4`-byte
//! chunks and a single short write shears every subsequent frame. So each
//! `write-frame!` verifies the byte count against the writer's geometry and
//! refuses a mismatch rather than producing a video that is subtly wrong.

use std::any::Any;
use std::io::{Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use cljrs_value::{Keyword, MapValue, Resource, Value, ValueError, ValueResult};

/// How a finished encode turned out.
#[derive(Clone, Debug)]
pub struct Outcome {
    pub exit: i32,
    pub stderr: String,
    pub frames: u64,
}

struct Open {
    child: Child,
    stdin: Option<ChildStdin>,
    stderr_buf: Arc<Mutex<String>>,
    stderr_thread: Option<JoinHandle<()>>,
}

enum State {
    Open(Open),
    Closed(Outcome),
}

/// An `ffmpeg` child process fed raw RGBA frames over stdin.
pub struct VideoWriter {
    pub path: String,
    pub width: u32,
    pub height: u32,
    frame_bytes: usize,
    frames: AtomicU64,
    state: Mutex<State>,
}

impl std::fmt::Debug for VideoWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "VideoWriter({} {}x{} frames={})",
            self.path,
            self.width,
            self.height,
            self.frames.load(Ordering::Relaxed)
        )
    }
}

const POISONED: &str = "video-writer lock poisoned by a panic in another thread";

/// How the encoder is configured, already resolved from the options map.
pub struct WriterSpec {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub codec: String,
    pub pix_fmt: String,
    pub crf: Option<i64>,
    pub preset: Option<String>,
    pub loglevel: String,
    pub overwrite: bool,
    pub extra: Vec<String>,
}

impl VideoWriter {
    /// Spawn the encoder.
    pub fn spawn(path: &str, spec: &WriterSpec) -> Result<Self, String> {
        if spec.width == 0 || spec.height == 0 {
            return Err("a video writer needs a non-zero :width and :height".to_string());
        }
        // The 4:2:0 chroma planes are half-resolution, so an odd dimension has
        // no valid encoding. ffmpeg's own error for this is buried 30 lines
        // into stderr; say it up front instead.
        if spec.pix_fmt.starts_with("yuv42") && (spec.width % 2 == 1 || spec.height % 2 == 1) {
            return Err(format!(
                "{} needs even dimensions, got {}x{} — round up or pick another :pix-fmt",
                spec.pix_fmt, spec.width, spec.height
            ));
        }

        let mut args: Vec<String> = Vec::new();
        if spec.overwrite {
            args.push("-y".into());
        } else {
            args.push("-n".into());
        }
        args.extend([
            "-hide_banner".into(),
            "-loglevel".into(),
            spec.loglevel.clone(),
        ]);
        // Input: raw straight-alpha RGBA, exactly what `cljrs.raster/rgba-bytes`
        // hands back.
        args.extend([
            "-f".into(),
            "rawvideo".into(),
            "-pixel_format".into(),
            "rgba".into(),
            "-video_size".into(),
            format!("{}x{}", spec.width, spec.height),
            "-framerate".into(),
            format!("{}", spec.fps),
            "-i".into(),
            "-".into(),
        ]);
        args.push("-an".into()); // no audio stream from a frame pipe
        args.extend(["-c:v".into(), spec.codec.clone()]);
        args.extend(["-pix_fmt".into(), spec.pix_fmt.clone()]);
        if let Some(crf) = spec.crf {
            args.extend(["-crf".into(), crf.to_string()]);
        }
        if let Some(p) = &spec.preset {
            args.extend(["-preset".into(), p.clone()]);
        }
        args.extend(["-r".into(), format!("{}", spec.fps)]);
        args.extend(spec.extra.iter().cloned());
        args.push(path.to_string());

        let bin = crate::proc::ffmpeg_bin();
        let mut child = Command::new(&bin)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot start `{bin}`: {e}"))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "ffmpeg gave us no stdin pipe".to_string())?;

        // Drain stderr on its own thread. ffmpeg is chatty even at `error`
        // level, and an unread pipe fills and blocks the encoder mid-write —
        // a deadlock that only shows up on long renders.
        let stderr_buf = Arc::new(Mutex::new(String::new()));
        let stderr_thread = child.stderr.take().map(|mut err| {
            let buf = Arc::clone(&stderr_buf);
            std::thread::spawn(move || {
                let mut s = String::new();
                let _ = err.read_to_string(&mut s);
                if let Ok(mut guard) = buf.lock() {
                    guard.push_str(&s);
                }
            })
        });

        Ok(VideoWriter {
            path: path.to_string(),
            width: spec.width,
            height: spec.height,
            frame_bytes: (spec.width as usize) * (spec.height as usize) * 4,
            frames: AtomicU64::new(0),
            state: Mutex::new(State::Open(Open {
                child,
                stdin: Some(stdin),
                stderr_buf,
                stderr_thread,
            })),
        })
    }

    /// Bytes one frame must contain.
    pub fn frame_bytes(&self) -> usize {
        self.frame_bytes
    }

    pub fn frames_written(&self) -> u64 {
        self.frames.load(Ordering::Relaxed)
    }

    /// Write one raw RGBA frame.
    pub fn write_frame(&self, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() != self.frame_bytes {
            return Err(format!(
                "expected {} bytes for a {}x{} RGBA frame, got {}",
                self.frame_bytes,
                self.width,
                self.height,
                bytes.len()
            ));
        }
        let mut guard = self.state.lock().map_err(|_| POISONED.to_string())?;
        let open = match &mut *guard {
            State::Open(o) => o,
            State::Closed(_) => {
                return Err(format!("video writer for {} is already closed", self.path));
            }
        };
        let stdin = open
            .stdin
            .as_mut()
            .ok_or_else(|| "video writer stdin already dropped".to_string())?;
        match stdin.write_all(bytes) {
            Ok(()) => {
                self.frames.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            Err(e) => {
                // A broken pipe means ffmpeg died; its stderr says why.
                let tail = open
                    .stderr_buf
                    .lock()
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default();
                Err(if tail.is_empty() {
                    format!("writing a frame failed: {e}")
                } else {
                    format!("writing a frame failed: {e} — ffmpeg said: {tail}")
                })
            }
        }
    }

    /// Close the pipe, wait for the encoder, and report the outcome.
    ///
    /// Idempotent: a second call returns the same outcome without re-waiting.
    pub fn finish(&self) -> Result<Outcome, String> {
        let mut guard = self.state.lock().map_err(|_| POISONED.to_string())?;
        if let State::Closed(o) = &*guard {
            return Ok(o.clone());
        }
        let State::Open(open) = &mut *guard else {
            unreachable!("checked above")
        };

        // Dropping stdin sends EOF, which is how ffmpeg knows to flush and
        // finalize the container. Without it `wait` hangs forever.
        open.stdin.take();

        let status = open
            .child
            .wait()
            .map_err(|e| format!("waiting for ffmpeg failed: {e}"))?;
        if let Some(t) = open.stderr_thread.take() {
            let _ = t.join();
        }
        let stderr = open
            .stderr_buf
            .lock()
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        let outcome = Outcome {
            exit: status.code().unwrap_or(-1),
            stderr,
            frames: self.frames.load(Ordering::Relaxed),
        };
        *guard = State::Closed(outcome.clone());
        Ok(outcome)
    }
}

impl Resource for VideoWriter {
    fn close(&self) -> ValueResult<()> {
        self.finish().map(|_| ()).map_err(ValueError::Other)
    }

    fn is_closed(&self) -> bool {
        matches!(self.state.lock().as_deref(), Ok(State::Closed(_)))
    }

    fn resource_type(&self) -> &'static str {
        "video-writer"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// `{:exit n :frames n :error "…" :path "…"}` — the shape `close-writer!`
/// returns.
pub fn outcome_value(path: &str, o: &Outcome) -> Value {
    let kw = |n: &str| Value::keyword(Keyword::simple(n));
    Value::Map(MapValue::from_pairs(vec![
        (kw("path"), Value::string(path.to_string())),
        (kw("exit"), Value::Long(o.exit as i64)),
        (kw("frames"), Value::Long(o.frames as i64)),
        (
            kw("error"),
            if o.stderr.is_empty() {
                Value::Nil
            } else {
                Value::string(o.stderr.clone())
            },
        ),
    ]))
}
