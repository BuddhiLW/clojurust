//! Regular expressions: the engine wrapper behind `Value::Pattern`, plus the
//! stateful `Matcher` that `re-find`/`re-matches`/`re-seq` drive.
//!
//! Clojure's `#"…"` literal is a first-class value, so whichever engine is
//! selected is linked into *every* build — there is no way to opt out of regex
//! support and still read Clojure source. Two engines are selectable, and
//! `Pattern` is the seam between them:
//!
//! - `regex-full` (default) — the `regex` crate. Fast, linear-time, full
//!   Unicode character classes. Brings `regex-automata`, `regex-syntax`,
//!   `aho-corasick` and `memchr` with it.
//! - `small-regex` — the `regex-lite` crate: roughly a tenth of the size, no
//!   DFA/SIMD machinery. Materially slower on pathological patterns and it has
//!   **no Unicode character classes**, so this is a behaviour change and not
//!   only a size one. Measured on a stripped release build of the interpreter
//!   plus `clojure.core`/`clojure.string` with `deps` off: 3.68 MB of `.text`
//!   down to 2.89 MB, 5.77 MB of binary down to 4.10 MB.
//!
//! Features are additive, so `regex-full` wins when both are enabled: a build
//! that pulls in one dependent asking for the small engine and another taking
//! the default gets the more capable engine rather than a silent semantic
//! downgrade. Cargo also unions features across every edge to a package, so one
//! internal edge left at its defaults would re-enable `regex-full` for the whole
//! graph and no second, direct dependency could switch it off again. Every
//! workspace crate therefore takes its own internal dependencies with default
//! features off and re-exports both features (along with `deps`, which those
//! defaults used to carry), so `default-features = false` at the embedder's own
//! edge is enough to select the small engine.

use crate::{PersistentVector, Value};
use cljrs_gc::{GcPtr, GcVisitor, MarkVisitor, Trace};
use std::borrow::Cow;
use std::fmt;
use std::sync::{Mutex, OnceLock};

#[cfg(feature = "regex-full")]
use regex as engine;
#[cfg(all(feature = "small-regex", not(feature = "regex-full")))]
use regex_lite as engine;

#[cfg(not(any(feature = "regex-full", feature = "small-regex")))]
compile_error!(
    "cljrs-value needs a regex engine: keep the default `regex-full` feature, \
     or enable `small-regex` to use regex-lite instead."
);

/// A compiled regular expression — the payload of `Value::Pattern`.
///
/// Every regex operation in the runtime goes through this type, so swapping
/// engines is confined to this file.
#[derive(Debug, Clone)]
pub struct Pattern {
    re: engine::Regex,
    /// The `\A(?:…)\z` form, compiled on the first `re-matches` against this
    /// pattern and shared by every later one. `Some(None)` records a
    /// compilation that failed. Neither engine exposes an anchored search, and
    /// filtering an unanchored one is not the same thing: both pick the
    /// leftmost-first match and would offer `a` for `a|ab`, so the anchors have
    /// to be inside the automaton where they can steer the match.
    anchored: OnceLock<Option<engine::Regex>>,
}

/// A pattern that failed to compile. Engine-independent so that callers do not
/// name `regex::Error` or `regex_lite::Error`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternError(String);

impl fmt::Display for PatternError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PatternError {}

impl Pattern {
    /// Compile `pattern`.
    pub fn new(pattern: &str) -> Result<Pattern, PatternError> {
        engine::Regex::new(pattern)
            .map(|re| Pattern {
                re,
                anchored: OnceLock::new(),
            })
            .map_err(|e| PatternError(e.to_string()))
    }

    /// The pattern source, as written in the `#"…"` literal.
    pub fn as_str(&self) -> &str {
        self.re.as_str()
    }

    /// Leftmost match anywhere in `haystack`.
    pub fn captures<'h>(&self, haystack: &'h str) -> Option<Captures<'h>> {
        self.re.captures(haystack).map(Captures)
    }

    /// Match the whole of `haystack`, or nothing — Java's `Matcher.matches()`,
    /// which Clojure's `re-matches` delegates to.
    ///
    /// Anchored inside the pattern rather than by checking the span of an
    /// unanchored match, because the two differ wherever the engine's
    /// leftmost-first preference picks a shorter alternative: `a|ab` prefers
    /// `a`, and `.*?x?` prefers the empty match, neither of which reaches the
    /// end of the haystack even though a full match exists. With `\A`/`\z` in
    /// the automaton the engine only ever offers a whole-haystack match.
    pub fn captures_full<'h>(&self, haystack: &'h str) -> Option<Captures<'h>> {
        match self.anchored() {
            Some(re) => re.captures(haystack).map(Captures),
            // Anchoring an already-valid pattern can only fail on a size or
            // nesting limit. Fall back to the span of an unanchored match: it
            // still never accepts a partial one, it can only miss a full match
            // the engine resolved in favour of a shorter alternative.
            None => self
                .captures(haystack)
                .filter(|cap| cap.start() == 0 && cap.end() == haystack.len()),
        }
    }

    /// The anchored twin of this pattern, compiled once. `(?:…)` keeps the
    /// group numbering — and any leading inline flags — of the original.
    fn anchored(&self) -> Option<&engine::Regex> {
        self.anchored
            .get_or_init(|| engine::Regex::new(&format!(r"\A(?:{})\z", self.as_str())).ok())
            .as_ref()
    }

    /// Leftmost match at or after byte offset `start`. Look-around still sees
    /// the text before `start`, which is what makes this the right primitive
    /// for stepping a `Matcher` forward.
    pub fn captures_at<'h>(&self, haystack: &'h str, start: usize) -> Option<Captures<'h>> {
        self.re.captures_at(haystack, start).map(Captures)
    }

    /// Replace the leftmost match in `haystack`; `$1`-style references in
    /// `replacement` expand to capture groups.
    pub fn replace<'h>(&self, haystack: &'h str, replacement: &str) -> Cow<'h, str> {
        self.re.replace(haystack, replacement)
    }

    /// Replace every non-overlapping match in `haystack`.
    pub fn replace_all<'h>(&self, haystack: &'h str, replacement: &str) -> Cow<'h, str> {
        self.re.replace_all(haystack, replacement)
    }

    /// Split `haystack` around each match.
    pub fn split<'h>(&self, haystack: &'h str) -> impl Iterator<Item = &'h str> {
        self.re.split(haystack)
    }

    /// Split `haystack` around each match, yielding at most `limit` pieces;
    /// the last piece holds the unsplit remainder.
    pub fn splitn<'h>(&self, haystack: &'h str, limit: usize) -> impl Iterator<Item = &'h str> {
        self.re.splitn(haystack, limit)
    }
}

impl fmt::Display for Pattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Trace for Pattern {
    fn trace(&self, _: &mut MarkVisitor) {}
}

/// A successful match and its capture groups.
///
/// Borrows the haystack, so it never escapes the call that produced it —
/// `MatchResult` is the owned form that outlives a match.
#[derive(Debug)]
pub struct Captures<'h>(engine::Captures<'h>);

impl<'h> Captures<'h> {
    /// Text of the whole match (group 0).
    pub fn full(&self) -> &'h str {
        self.whole().as_str()
    }

    /// Byte offset where the whole match begins. Non-zero whenever the
    /// leftmost match starts part-way into the haystack.
    pub fn start(&self) -> usize {
        self.whole().start()
    }

    /// Byte offset just past the whole match — where the next search starts.
    pub fn end(&self) -> usize {
        self.whole().end()
    }

    /// Number of groups, group 0 included; always at least 1.
    pub fn group_count(&self) -> usize {
        self.0.len()
    }

    /// Every group in order, starting with group 0. `None` marks a group that
    /// did not participate in the match.
    pub fn groups(&self) -> impl Iterator<Item = Option<&'h str>> + '_ {
        self.0.iter().map(|g| g.map(|m| m.as_str()))
    }

    /// Group 0, which always participates in a successful match. Not
    /// `Captures::get_match`, which `regex-lite` does not have.
    fn whole(&self) -> engine::Match<'h> {
        self.0
            .get(0)
            .expect("group 0 always participates in a successful match")
    }
}

#[derive(Debug, Clone)]
pub enum MatchPhase {
    New,
    /// A match is available from `capture()`; the payload is the byte offset
    /// the next search resumes from, which is past the end of the haystack once
    /// there is nothing left to search.
    Matching(usize),
    Complete,
}

#[derive(Debug, Clone)]
struct MatcherState {
    phase: MatchPhase,
    last_match: Option<MatchResult>,
}

#[derive(Debug)]
pub struct Matcher {
    pub pattern: GcPtr<Pattern>,
    haystack: GcPtr<String>,
    state: Mutex<MatcherState>,
    match_all: bool,
}

#[derive(Debug, Clone)]
pub struct MatchResult {
    pub full: String,
    pub groups: Vec<Option<String>>,
}

impl Clone for Matcher {
    fn clone(&self) -> Matcher {
        let state = self.state.lock().unwrap().clone();
        Matcher {
            pattern: self.pattern.clone(),
            haystack: self.haystack.clone(),
            state: Mutex::new(state.clone()),
            match_all: self.match_all,
        }
    }
}

impl Trace for Matcher {
    fn trace(&self, visitor: &mut MarkVisitor) {
        visitor.visit(&self.pattern);
        visitor.visit(&self.haystack);
    }
}

impl Matcher {
    pub fn new(pattern: Pattern, source: String, match_all: bool) -> Self {
        Self::from_ptr(GcPtr::new(pattern), source, match_all)
    }

    /// As `new`, but sharing an already-allocated pattern — a `#"…"` literal
    /// keeps its `Pattern` across evaluations, so its anchored form (see
    /// `Pattern::captures_full`) is compiled once rather than per call.
    pub fn from_ptr(pattern: GcPtr<Pattern>, source: String, match_all: bool) -> Self {
        Self {
            pattern,
            haystack: GcPtr::new(source),
            state: Mutex::new(MatcherState {
                phase: MatchPhase::New,
                last_match: None,
            }),
            match_all,
        }
    }

    pub fn next(&self) -> MatchPhase {
        let mut state = self.state.lock().unwrap();
        let pattern = self.pattern.get();
        let haystack = self.haystack.get();
        match state.phase {
            MatchPhase::New => {
                // `match_all` is `re-matches`: the whole haystack has to match,
                // so the search itself is anchored rather than filtered after
                // the fact.
                let cap = if self.match_all {
                    pattern.captures_full(haystack)
                } else {
                    pattern.captures(haystack)
                };
                *state = Self::step(cap, haystack);
            }
            MatchPhase::Matching(n) => {
                // A `match_all` matcher yields at most one match: it spans the
                // whole haystack, so there is nothing left to step to. Without
                // this, patterns that can match empty (`a*`) would keep
                // handing back the zero-width match at the end.
                let cap = if self.match_all || n > haystack.len() {
                    None
                } else {
                    pattern.captures_at(haystack, n)
                };
                *state = Self::step(cap, haystack);
            }
            MatchPhase::Complete => {}
        }
        state.phase.clone()
    }

    /// The state a search lands in: `Matching` while matches remain, `Complete`
    /// once one comes up empty.
    fn step(cap: Option<Captures<'_>>, haystack: &str) -> MatcherState {
        match cap {
            Some(cap) => MatcherState {
                phase: MatchPhase::Matching(resume_from(&cap, haystack)),
                last_match: Some(MatchResult::new(&cap)),
            },
            None => MatcherState {
                phase: MatchPhase::Complete,
                last_match: None,
            },
        }
    }

    pub fn capture(&self) -> Option<MatchResult> {
        let state = self.state.lock().unwrap();
        state.last_match.clone()
    }

    pub fn phase(&self) -> MatchPhase {
        self.state.lock().unwrap().phase.clone()
    }
}

/// Where the search after `cap` resumes, following Java's `Matcher.find`: the
/// end of the match, bumped past one character when the match was zero-width.
/// Without the bump an empty match is found at the same offset forever, so
/// `(re-seq #"a*" "aaa")` never terminates. The result can land one past the
/// end of the haystack, which is how the matcher knows it is done.
fn resume_from(cap: &Captures<'_>, haystack: &str) -> usize {
    let end = cap.end();
    if cap.start() != end {
        return end;
    }
    // One *character*, not one byte: `captures_at` panics off a UTF-8 boundary.
    match haystack[end..].chars().next() {
        Some(c) => end + c.len_utf8(),
        None => end + 1,
    }
}

impl MatchResult {
    pub fn new(cap: &Captures<'_>) -> Self {
        Self {
            full: cap.full().to_string(),
            groups: cap.groups().map(|g| g.map(|e| e.to_string())).collect(),
        }
    }

    pub fn to_value(&self) -> Value {
        if self.groups.len() == 1 || self.groups.iter().skip(1).all(|g| g.is_none()) {
            Value::Str(GcPtr::new(self.full.to_string()))
        } else {
            let groups: Vec<Value> = self
                .groups
                .iter()
                .map(|g| match g {
                    Some(m) => Value::Str(GcPtr::new(m.to_string())),
                    None => Value::Nil,
                })
                .collect();
            Value::Vector(GcPtr::new(PersistentVector::from_iter(groups)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Value` is shared across threads, so the engine's `Regex` must be too.
    #[test]
    fn pattern_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Pattern>();
    }

    #[test]
    fn captures_expose_groups_in_order() {
        let p = Pattern::new(r"(\d+)-(\d+)").unwrap();
        let cap = p.captures("x 12-345 y").unwrap();
        assert_eq!(cap.full(), "12-345");
        assert_eq!(cap.group_count(), 3);
        assert_eq!(cap.end(), 8);
        assert_eq!(
            cap.groups().collect::<Vec<_>>(),
            vec![Some("12-345"), Some("12"), Some("345")]
        );
    }

    #[test]
    fn non_participating_group_is_none() {
        let p = Pattern::new(r"(a)|(b)").unwrap();
        let cap = p.captures("b").unwrap();
        assert_eq!(
            cap.groups().collect::<Vec<_>>(),
            vec![Some("b"), None, Some("b")]
        );
    }

    #[test]
    fn captures_at_resumes_after_a_match() {
        let p = Pattern::new(r"\d+").unwrap();
        let cap = p.captures_at("a1 b22", 2).unwrap();
        assert_eq!(cap.full(), "22");
    }

    #[test]
    fn invalid_pattern_reports_the_engine_message() {
        let err = Pattern::new(r"(").unwrap_err();
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn split_replace_and_display() {
        let p = Pattern::new(r",\s*").unwrap();
        assert_eq!(p.split("a, b,c").collect::<Vec<_>>(), vec!["a", "b", "c"]);
        assert_eq!(p.splitn("a, b,c", 2).collect::<Vec<_>>(), vec!["a", "b,c"]);
        assert_eq!(p.replace("a, b,c", "|"), "a|b,c");
        assert_eq!(p.replace_all("a, b,c", "|"), "a|b|c");
        assert_eq!(p.as_str(), r",\s*");
        assert_eq!(p.to_string(), r",\s*");
    }

    /// `re-matches` semantics: the whole haystack has to match, and a
    /// group-less pattern is no different from one with groups.
    /// One `re-matches` against `haystack`, as `builtin_re_matches` drives it.
    fn full_match(pattern: &str, haystack: &str) -> Option<MatchResult> {
        let m = Matcher::new(Pattern::new(pattern).unwrap(), haystack.to_string(), true);
        m.next();
        m.capture()
    }

    #[test]
    fn match_all_requires_the_whole_haystack() {
        for (pattern, haystack) in [
            (r"\d+", "42"),
            (r"\d+", "424"),
            (r"\d+", "4"),
            (r"a+", "aaa"),
            (r".*", "hello"),
        ] {
            assert_eq!(
                full_match(pattern, haystack).map(|c| c.full),
                Some(haystack.to_string()),
                "{pattern} should match all of {haystack}"
            );
        }

        let cap = full_match(r"(\d+)-(\d+)", "12-345").unwrap();
        assert_eq!(cap.full, "12-345");
        assert_eq!(
            cap.groups,
            vec![Some("12-345".into()), Some("12".into()), Some("345".into())]
        );

        // A match that stops short of the end, whatever the group count.
        assert!(full_match(r"(a)(b)", "abc").is_none());
        assert!(full_match(r"(a)", "ab").is_none());
        assert!(full_match(r"\d+", "42x").is_none());
        // …and one that starts part-way in.
        assert!(full_match(r"(a)(b)", "xab").is_none());
        assert!(full_match(r"\d+", "x42").is_none());
        // No match at all.
        assert!(full_match(r"\d+", "abc").is_none());
    }

    /// Both engines are leftmost-first, so an unanchored search offers the
    /// shorter alternative and a span check would reject a haystack that does
    /// match in full. Anchoring the pattern itself is what makes these work.
    #[test]
    fn match_all_beats_leftmost_first_preference() {
        assert_eq!(full_match(r"a|ab", "ab").map(|c| c.full), Some("ab".into()));
        assert_eq!(
            full_match(r"(a|ab)(c|bc)", "abc").map(|c| c.full),
            Some("abc".into())
        );
        // Lazy repetition prefers to stop early; `\z` forces it to the end.
        assert_eq!(
            full_match(r".*?", "hello").map(|c| c.full),
            Some("hello".into())
        );
        assert_eq!(
            full_match(r"(\w+?)(\d*)", "ab12").map(|c| c.groups),
            Some(vec![
                Some("ab12".into()),
                Some("ab".into()),
                Some("12".into())
            ])
        );
        // The unanchored search these replace really does come up short.
        let p = Pattern::new(r"a|ab").unwrap();
        assert_eq!(p.captures("ab").unwrap().full(), "a");
    }

    /// Anchoring wraps the source in `(?:…)`, which must not renumber groups or
    /// swallow a leading inline flag.
    #[test]
    fn match_all_preserves_groups_and_inline_flags() {
        let cap = full_match(r"(?i)(a)(b)", "AB").unwrap();
        assert_eq!(cap.full, "AB");
        assert_eq!(
            cap.groups,
            vec![Some("AB".into()), Some("A".into()), Some("B".into())]
        );
        // An empty pattern matches only an empty haystack.
        assert_eq!(full_match(r"", "").map(|c| c.full), Some(String::new()));
        assert!(full_match(r"", "a").is_none());
    }

    /// A successful `match_all` search consumes the haystack, so the matcher
    /// has nothing left to yield — `a*` must not keep offering the zero-width
    /// match at the end.
    #[test]
    fn match_all_yields_at_most_one_match() {
        let m = Matcher::new(Pattern::new(r"a*").unwrap(), "aaa".to_string(), true);
        assert!(matches!(m.next(), MatchPhase::Matching(3)));
        assert_eq!(m.capture().unwrap().full, "aaa");
        assert!(matches!(m.next(), MatchPhase::Complete));
        assert!(m.capture().is_none());
        assert!(matches!(m.next(), MatchPhase::Complete));
    }

    /// A rejected `match_all` search has nowhere left to step, so it must land
    /// in `Complete` rather than sitting in `New` forever.
    #[test]
    fn match_all_completes_when_the_match_is_partial() {
        let m = Matcher::new(Pattern::new(r"(a)").unwrap(), "ab".to_string(), true);
        assert!(matches!(m.next(), MatchPhase::Complete));
        assert!(m.capture().is_none());

        let mut steps = 0;
        while let MatchPhase::New | MatchPhase::Matching(_) = m.next() {
            steps += 1;
            assert!(steps < 10, "matcher never reached a terminal state");
        }
    }

    /// Every match a matcher yields, with a bound so a regression fails instead
    /// of hanging the suite.
    fn drain(pattern: &str, haystack: &str) -> Vec<String> {
        let m = Matcher::new(Pattern::new(pattern).unwrap(), haystack.to_string(), false);
        let mut found = Vec::new();
        while let MatchPhase::Matching(_) = m.next() {
            found.push(m.capture().unwrap().full);
            assert!(
                found.len() < 16,
                "matcher never reached Complete: {found:?}"
            );
        }
        assert!(matches!(m.phase(), MatchPhase::Complete));
        found
    }

    /// A zero-width match is found at the same offset forever unless the search
    /// moves on. Java's `find` bumps by one character after an empty match, so
    /// `#"a*"` terminates and yields the trailing empty match Clojure does.
    #[test]
    fn zero_width_matches_advance_and_terminate() {
        assert_eq!(drain(r"a*", "aaa"), vec!["aaa", ""]);
        assert_eq!(drain(r"a*", "bab"), vec!["", "a", "", ""]);
        assert_eq!(drain(r"", "ab"), vec!["", "", ""]);
        assert_eq!(drain(r"", ""), vec![""]);
        assert_eq!(drain(r"x*", "ab"), vec!["", "", ""]);
    }

    /// The bump is one character, not one byte: a byte-sized step would land
    /// inside `é` and panic in `captures_at`.
    #[test]
    fn zero_width_advance_respects_utf8_boundaries() {
        assert_eq!(drain(r"x*", "é"), vec!["", ""]);
        assert_eq!(drain(r"x*", "日本"), vec!["", "", ""]);
        assert_eq!(drain(r"é*", "éé"), vec!["éé", ""]);
    }

    #[test]
    fn matcher_walks_every_match_then_completes() {
        let p = Pattern::new(r"\d+").unwrap();
        let m = Matcher::new(p, "a1 b22 c333".to_string(), false);

        let mut found = Vec::new();
        while let MatchPhase::Matching(_) = m.next() {
            found.push(m.capture().unwrap().full);
        }
        assert_eq!(found, vec!["1", "22", "333"]);
        assert!(matches!(m.phase(), MatchPhase::Complete));
        assert!(m.capture().is_none());
    }
}
