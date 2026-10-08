//! Host-interface methods on `deftype` / `reify` / `proxy` instances.
//!
//! On the JVM a datatype may implement a Java interface (`Object`,
//! `clojure.lang.Indexed`, `ThreadLocal`, ...) as well as protocols. cljrs has
//! no classes, so those method bodies are kept in one hidden protocol table,
//! keyed by the instance's type tag like any protocol impl. Two consumers read
//! it: `.method` interop on an instance, and core hooks that the JVM reaches
//! through an interface (`nth` through `Indexed`).
//!
//! `clojure.test.check.rose-tree/RoseTree` implements `clojure.lang.Indexed`,
//! and `clojure.test.check.random` keeps its RNG in a `ThreadLocal` proxy.

use std::sync::Arc;

use crate::env::env::GlobalEnv;
use cljrs_gc::GcPtr;
use cljrs_value::{Protocol, Value};

/// The var in `clojure.core` holding the table.
const TABLE_VAR: &str = "__host-methods__";

/// True for a symbol naming a host class or interface rather than a protocol.
pub fn is_host_interface(s: &str) -> bool {
    matches!(
        s,
        "Object" | "java.lang.Object" | "ThreadLocal" | "java.lang.ThreadLocal"
    ) || s.starts_with("clojure.lang.")
        || s.starts_with("java.")
}

/// The host-method table, created on first use.
pub fn table(globals: &Arc<GlobalEnv>) -> GcPtr<Protocol> {
    if let Some(Value::Protocol(p)) = globals.lookup_in_ns("clojure.core", TABLE_VAR) {
        return p;
    }
    let p = GcPtr::new(Protocol::new(
        Arc::from("HostMethods"),
        Arc::from("clojure.core"),
        Vec::new(),
        false,
    ));
    globals.intern("clojure.core", Arc::from(TABLE_VAR), Value::Protocol(p.clone()));
    p
}

/// The host method `name` registered for the dispatch tag `tag`, if any.
pub fn lookup(globals: &Arc<GlobalEnv>, tag: &str, name: &str) -> Option<Value> {
    let Some(Value::Protocol(p)) = globals.lookup_in_ns("clojure.core", TABLE_VAR) else {
        return None;
    };
    let impls = p.get().impls.lock().unwrap();
    impls.get(tag).and_then(|m| m.get(name)).cloned()
}

/// Look up `name` for an instance from inside a native builtin (no `Env` in
/// hand), using the active eval context's globals.
pub fn lookup_in_context(tag: &str, name: &str) -> Option<Value> {
    let (globals, _) = crate::env::callback::capture_eval_context()?;
    lookup(&globals, tag, name)
}
