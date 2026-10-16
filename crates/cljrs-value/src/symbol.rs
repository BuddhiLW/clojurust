use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::Arc;

/// An interned Clojure symbol, optionally namespace-qualified, optionally
/// pinned to a git commit via the `@<hash>` suffix syntax.
///
/// `"foo"`              → `Symbol { namespace: None,              name: "foo", version: None }`
/// `"ns/name"`          → `Symbol { namespace: Some("ns"),        name: "name", version: None }`
/// `"my-fn@abc1234"`    → `Symbol { namespace: None,              name: "my-fn", version: Some("abc1234") }`
/// `"ns/fn@abc1234"`    → `Symbol { namespace: Some("ns"),        name: "fn", version: Some("abc1234") }`
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Symbol {
    pub namespace: Option<Arc<str>>,
    pub name: Arc<str>,
    /// Git commit hash suffix, present when the symbol was written as `name@hash`.
    pub version: Option<Arc<str>>,
}

impl Symbol {
    /// Unqualified, unversioned symbol.
    pub fn simple(name: impl Into<Arc<str>>) -> Self {
        Self {
            namespace: None,
            name: name.into(),
            version: None,
        }
    }

    /// Namespace-qualified, unversioned symbol.
    pub fn qualified(ns: impl Into<Arc<str>>, name: impl Into<Arc<str>>) -> Self {
        Self {
            namespace: Some(ns.into()),
            name: name.into(),
            version: None,
        }
    }

    /// Parse a symbol from a string of the form `"ns/name"`, `"name"`,
    /// `"name@hash"`, or `"ns/name@hash"`.
    ///
    /// The `@` version suffix is detected in the *name* portion (after any `/`
    /// split).  A bare `"/"` remains an unqualified symbol as before.
    ///
    /// Memoized per thread. Parsing is pure, but it allocates one `Arc<str>`
    /// per component, and the callers that dominate are conversions that see
    /// the *same* text over and over: `form_to_value` re-derives every symbol
    /// in a form tree each time a quoted form is evaluated or a macro call is
    /// expanded, and `macroexpand` runs the latter to a fixed point. A cache
    /// hit is three refcount bumps against three allocations plus two scans.
    pub fn parse(s: &str) -> Self {
        INTERNED
            .try_with(|table| {
                if let Some(sym) = table.borrow().get(s).cloned() {
                    return sym;
                }
                let sym = Self::parse_uncached(s);
                let mut table = table.borrow_mut();
                // `gensym` mints a fresh name per call, so an uncapped table
                // would grow without bound in a long-running program. Past the
                // cap parsing still answers, just uncached; the entries already
                // in are the ones a program returns to.
                if table.len() < INTERN_CAP {
                    table.insert(Box::from(s), sym.clone());
                }
                sym
            })
            // During thread teardown the table is already gone.
            .unwrap_or_else(|_| Self::parse_uncached(s))
    }

    /// [`Symbol::parse`] without the memo table.
    fn parse_uncached(s: &str) -> Self {
        // Split namespace qualifier on the first `/`.
        let (ns_part, name_part) = match s.find('/') {
            Some(idx) if idx > 0 && idx < s.len() - 1 => (Some(&s[..idx]), &s[idx + 1..]),
            _ => (None, s),
        };

        // Split version suffix on the last `@` in the name portion, accepting
        // only valid commit hashes (7–40 hex characters).
        let (base_name, version) = split_version(name_part);

        Symbol {
            namespace: ns_part.map(Arc::from),
            name: Arc::from(base_name),
            version: version.map(Arc::from),
        }
    }

    /// The unversioned, fully-qualified string: `"ns/name"` or `"name"`.
    pub fn full_name(&self) -> String {
        match &self.namespace {
            Some(ns) => format!("{}/{}", ns, self.name),
            None => self.name.to_string(),
        }
    }

    /// The display string including the version suffix if present:
    /// `"ns/name@hash"` or `"name@hash"` or `"name"`.
    pub fn versioned_name(&self) -> String {
        match (&self.namespace, &self.version) {
            (Some(ns), Some(v)) => format!("{}/{}@{}", ns, self.name, v),
            (Some(ns), None) => format!("{}/{}", ns, self.name),
            (None, Some(v)) => format!("{}@{}", self.name, v),
            (None, None) => self.name.to_string(),
        }
    }
}

/// Upper bound on memoized parses per thread. See [`Symbol::parse`].
const INTERN_CAP: usize = 4096;

thread_local! {
    /// Memo table behind [`Symbol::parse`], keyed by the exact source text.
    ///
    /// Thread-local rather than shared: the table is written on nearly every
    /// miss, so one lock would serialize symbol parsing across the tiering
    /// threads to buy sharing that a per-thread table does not need.
    static INTERNED: RefCell<HashMap<Box<str>, Symbol, BuildHasherDefault<FnvHasher>>> =
        RefCell::new(HashMap::default());
}

/// FNV-1a, for the [`INTERNED`] table only.
///
/// Symbol text is short and this table is read on a hot path, where SipHash's
/// per-call setup costs more than the collision resistance is worth: the keys
/// are program identifiers, not adversarial input.
#[derive(Debug)]
pub struct FnvHasher(u64);

impl Default for FnvHasher {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Hasher for FnvHasher {
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= u64::from(*b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

/// Split `name_part` into `(base, Some(hash))` if the last `@` is followed by
/// a valid commit hash (7–40 hex chars), otherwise return `(name_part, None)`.
pub fn split_version(name_part: &str) -> (&str, Option<&str>) {
    if let Some(at_pos) = name_part.rfind('@') {
        let candidate = &name_part[at_pos + 1..];
        if is_commit_hash(candidate) {
            return (&name_part[..at_pos], Some(candidate));
        }
    }
    (name_part, None)
}

/// Returns `true` if `s` could be an abbreviated or full git commit hash
/// (7–40 lowercase or uppercase hex characters).
pub fn is_commit_hash(s: &str) -> bool {
    (7..=40).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

impl std::fmt::Display for Symbol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.versioned_name())
    }
}

impl cljrs_gc::Trace for Symbol {
    fn trace(&self, _: &mut cljrs_gc::MarkVisitor) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple() {
        let s = Symbol::simple("foo");
        assert_eq!(s.name.as_ref(), "foo");
        assert!(s.namespace.is_none());
        assert!(s.version.is_none());
        assert_eq!(s.full_name(), "foo");
    }

    #[test]
    fn test_qualified() {
        let s = Symbol::qualified("clojure.core", "map");
        assert_eq!(s.full_name(), "clojure.core/map");
        assert!(s.version.is_none());
    }

    #[test]
    fn test_parse_unversioned() {
        assert_eq!(Symbol::parse("foo"), Symbol::simple("foo"));
        assert_eq!(Symbol::parse("a/b"), Symbol::qualified("a", "b"));
        assert_eq!(Symbol::parse("/").namespace, None);
    }

    #[test]
    fn test_parse_versioned_simple() {
        let s = Symbol::parse("my-fn@abc1234");
        assert_eq!(s.name.as_ref(), "my-fn");
        assert!(s.namespace.is_none());
        assert_eq!(s.version.as_deref(), Some("abc1234"));
        assert_eq!(s.versioned_name(), "my-fn@abc1234");
    }

    #[test]
    fn test_parse_versioned_qualified() {
        let s = Symbol::parse("my.ns/my-fn@abc1234");
        assert_eq!(s.namespace.as_deref(), Some("my.ns"));
        assert_eq!(s.name.as_ref(), "my-fn");
        assert_eq!(s.version.as_deref(), Some("abc1234"));
    }

    #[test]
    fn test_at_without_valid_hash_is_part_of_name() {
        // "@" followed by fewer than 7 hex chars is not a version.
        let s = Symbol::parse("my-fn@abc");
        assert_eq!(s.name.as_ref(), "my-fn@abc");
        assert!(s.version.is_none());
    }

    #[test]
    fn test_at_followed_by_non_hex_is_part_of_name() {
        let s = Symbol::parse("my-fn@not-hex");
        assert_eq!(s.name.as_ref(), "my-fn@not-hex");
        assert!(s.version.is_none());
    }

    /// The memo table keys on the whole source text, so texts that share a
    /// component must not answer for each other.
    #[test]
    fn memoized_parse_agrees_with_the_uncached_parse() {
        for s in [
            "foo",
            "a/b",
            "b",
            "/",
            "a/b@abc1234",
            "b@abc1234",
            "my-fn@abc",
            "clojure.core/map",
            "map",
        ] {
            // Twice, so the second answer is the cached one.
            assert_eq!(Symbol::parse(s), Symbol::parse_uncached(s), "first {s}");
            assert_eq!(Symbol::parse(s), Symbol::parse_uncached(s), "cached {s}");
        }
    }

    /// Past the cap parsing still answers correctly, just without caching.
    #[test]
    fn parse_is_correct_beyond_the_intern_cap() {
        for i in 0..(INTERN_CAP + 64) {
            let text = format!("gen-{i}/sym-{i}@abc1234");
            let sym = Symbol::parse(&text);
            assert_eq!(sym, Symbol::parse_uncached(&text));
            assert_eq!(sym.namespace.as_deref(), Some(format!("gen-{i}").as_str()));
            assert_eq!(sym.version.as_deref(), Some("abc1234"));
        }
    }
}
