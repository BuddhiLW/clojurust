// Every `do_*` connect/response driver returns `cljrs_runtime`'s `EvalError` by
// value, as the rest of the workspace does (`cljrs-runtime/src/lib.rs` allows
// this lint crate-wide).  Clippy only began flagging `async fn`s for it in 1.98
// — the seven sites here are unchanged from when they passed under 1.94.
#![allow(clippy::result_large_err)]

//! Networking for clojurust — TCP, Unix, UDP, TLS over core.async channels.
//!
//! # Usage
//!
//! ```rust,ignore
//! let runtime = cljrs_runtime::Runtime::builder()
//!     .execution_mode(cljrs_runtime::ExecutionMode::Tiered)
//!     .build()?;
//! cljrs_stdlib::install(&runtime);
//! let globals = runtime.globals().clone();
//! cljrs_async::init(&globals); // channels + executor (required)
//! cljrs_net::init(&globals);   // networking
//! ```
//!
//! Like `cljrs-async` and `cljrs-io`, requires a Tokio `current_thread` +
//! `LocalSet` executor running on the calling thread. The CLI links this crate
//! by default under the `net` feature (on by default, same as `async`).

use std::sync::Arc;

use cljrs_async::load_source;

pub mod frame;
pub mod h2;
pub mod h3;
mod pool_io;
pub mod quic;
pub mod quic_config;
pub mod tcp;
pub mod tls;
pub mod udp;
pub mod unix;

/// Clojure source for `clojure.rust.net.tcp`.
const NET_TCP_SOURCE: &str = include_str!("clojure_rust_net_tcp.cljrs");

/// Clojure source for the umbrella `clojure.rust.net` namespace.
const NET_SOURCE: &str = include_str!("clojure_rust_net.cljrs");

/// Clojure source for `clojure.rust.net.frame`.
const NET_FRAME_SOURCE: &str = include_str!("clojure_rust_net_frame.cljrs");

/// Clojure source for `clojure.rust.net.udp`.
const NET_UDP_SOURCE: &str = include_str!("clojure_rust_net_udp.cljrs");

/// Clojure source for `clojure.rust.net.tls`.
const NET_TLS_SOURCE: &str = include_str!("clojure_rust_net_tls.cljrs");

/// Clojure source for `clojure.rust.net.unix`.
const NET_UNIX_SOURCE: &str = include_str!("clojure_rust_net_unix.cljrs");

/// Clojure source for `clojure.rust.net.quic`.
const NET_QUIC_SOURCE: &str = include_str!("clojure_rust_net_quic.cljrs");

/// Clojure source for `clojure.rust.net.h3`.
const NET_H3_SOURCE: &str = include_str!("clojure_rust_net_h3.cljrs");

/// Clojure source for `clojure.rust.net.http2`.
const NET_HTTP2_SOURCE: &str = include_str!("clojure_rust_net_http2.cljrs");

pub const NS_TCP: &str = "clojure.rust.net.tcp";
pub const NS: &str = "clojure.rust.net";
pub const NS_FRAME: &str = "clojure.rust.net.frame";
pub const NS_UDP: &str = "clojure.rust.net.udp";
pub const NS_TLS: &str = "clojure.rust.net.tls";
pub const NS_UNIX: &str = "clojure.rust.net.unix";
pub const NS_QUIC: &str = "clojure.rust.net.quic";
pub const NS_H3: &str = "clojure.rust.net.h3";
pub const NS_HTTP2: &str = "clojure.rust.net.http2";

/// Register the networking namespaces.
///
/// Calls `cljrs_async::init` internally (idempotent) so callers only need to
/// call this one function. Requires a running `LocalSet` executor. Idempotent.
pub fn init(globals: &Arc<cljrs_runtime::env::env::GlobalEnv>) {
    cljrs_async::init(globals);

    if !globals.is_loaded(NS_TCP) {
        globals.get_or_create_ns(NS_TCP);
        globals.refer_core(NS_TCP);
        tcp::register(globals, NS_TCP);
        load_source(globals, NS_TCP, NET_TCP_SOURCE);
        globals.mark_loaded(NS_TCP);
    }

    if !globals.is_loaded(NS_FRAME) {
        globals.get_or_create_ns(NS_FRAME);
        globals.refer_core(NS_FRAME);
        frame::register(globals, NS_FRAME);
        load_source(globals, NS_FRAME, NET_FRAME_SOURCE);
        globals.mark_loaded(NS_FRAME);
    }

    if !globals.is_loaded(NS_UDP) {
        globals.get_or_create_ns(NS_UDP);
        globals.refer_core(NS_UDP);
        udp::register(globals, NS_UDP);
        load_source(globals, NS_UDP, NET_UDP_SOURCE);
        globals.mark_loaded(NS_UDP);
    }

    if !globals.is_loaded(NS_TLS) {
        globals.get_or_create_ns(NS_TLS);
        globals.refer_core(NS_TLS);
        tls::register(globals, NS_TLS);
        load_source(globals, NS_TLS, NET_TLS_SOURCE);
        globals.mark_loaded(NS_TLS);
    }

    if !globals.is_loaded(NS_UNIX) {
        globals.get_or_create_ns(NS_UNIX);
        globals.refer_core(NS_UNIX);
        unix::register(globals, NS_UNIX);
        load_source(globals, NS_UNIX, NET_UNIX_SOURCE);
        globals.mark_loaded(NS_UNIX);
    }

    if !globals.is_loaded(NS_QUIC) {
        globals.get_or_create_ns(NS_QUIC);
        globals.refer_core(NS_QUIC);
        quic::register(globals, NS_QUIC);
        load_source(globals, NS_QUIC, NET_QUIC_SOURCE);
        globals.mark_loaded(NS_QUIC);
    }

    if !globals.is_loaded(NS_H3) {
        globals.get_or_create_ns(NS_H3);
        globals.refer_core(NS_H3);
        h3::register(globals, NS_H3);
        load_source(globals, NS_H3, NET_H3_SOURCE);
        globals.mark_loaded(NS_H3);
    }

    if !globals.is_loaded(NS_HTTP2) {
        globals.get_or_create_ns(NS_HTTP2);
        globals.refer_core(NS_HTTP2);
        h2::register(globals, NS_HTTP2);
        load_source(globals, NS_HTTP2, NET_HTTP2_SOURCE);
        globals.mark_loaded(NS_HTTP2);
    }

    if !globals.is_loaded(NS) {
        globals.get_or_create_ns(NS);
        globals.refer_core(NS);
        load_source(globals, NS, NET_SOURCE);
        globals.mark_loaded(NS);
    }
}
