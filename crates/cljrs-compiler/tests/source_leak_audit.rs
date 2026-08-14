//! Property tests for `aot::audit_source` - the invariant behind
//! `cljrs compile --require-fully-compiled`.

use std::sync::Arc;

use cljrs_compiler::aot::{
    AotError, CompiledNamespace, OpacityPolicy, OpacityVerdict, SourceLeakChannel, audit_source,
};
use proptest::prelude::*;

fn ns_of(name: &str, preamble: &str) -> CompiledNamespace {
    CompiledNamespace {
        ns: Arc::from(name),
        init_symbol: Some(format!("__cljrs_ns_init_{name}")),
        preamble: preamble.to_string(),
    }
}

fn bundled_of(pairs: Vec<(String, String)>) -> Vec<(Arc<str>, String)> {
    pairs
        .into_iter()
        .map(|(n, s)| (Arc::from(n.as_str()), s))
        .collect()
}

/// Every non-empty source fragment, in every channel, weighed once.
fn expected_bytes(entry: &str, nss: &[CompiledNamespace], bundled: &[(Arc<str>, String)]) -> usize {
    entry.len()
        + nss.iter().map(|n| n.preamble.len()).sum::<usize>()
        + bundled.iter().map(|(_, s)| s.len()).sum::<usize>()
}

proptest! {
    /// Soundness: the audit is silent exactly when no readable text ships.
    /// A false negative here is a binary that leaks source while the gate
    /// reports success.
    #[test]
    fn empty_audit_iff_no_embedded_text(
        entry in "[a-z(); \n]{0,40}",
        preambles in prop::collection::vec("[a-z(); \n]{0,30}", 0..5),
        bundled in prop::collection::vec(("[a-z.]{1,8}", "[a-z(); \n]{0,30}"), 0..5),
    ) {
        let nss: Vec<_> = preambles.iter().enumerate()
            .map(|(i, p)| ns_of(&format!("ns{i}"), p)).collect();
        let bundled = bundled_of(bundled);

        let audit = audit_source(&entry, &nss, &bundled);
        let any_text = expected_bytes(&entry, &nss, &bundled) > 0;

        prop_assert_eq!(audit.is_clean(), !any_text);
    }

    /// Byte accounting is exact - no fragment is under-counted or double-counted.
    #[test]
    fn reported_bytes_equal_embedded_bytes(
        entry in "[a-z(); \n]{0,40}",
        preambles in prop::collection::vec("[a-z(); \n]{0,30}", 0..5),
        bundled in prop::collection::vec(("[a-z.]{1,8}", "[a-z(); \n]{0,30}"), 0..5),
    ) {
        let nss: Vec<_> = preambles.iter().enumerate()
            .map(|(i, p)| ns_of(&format!("ns{i}"), p)).collect();
        let bundled = bundled_of(bundled);

        let reported = audit_source(&entry, &nss, &bundled).total_bytes();

        prop_assert_eq!(reported, expected_bytes(&entry, &nss, &bundled));
    }

    /// One entry per non-empty fragment, attributed to the right channel.
    #[test]
    fn one_leak_per_nonempty_fragment(
        entry in "[a-z(); \n]{0,40}",
        preambles in prop::collection::vec("[a-z(); \n]{0,30}", 0..5),
        bundled in prop::collection::vec(("[a-z.]{1,8}", "[a-z(); \n]{0,30}"), 0..5),
    ) {
        let nss: Vec<_> = preambles.iter().enumerate()
            .map(|(i, p)| ns_of(&format!("ns{i}"), p)).collect();
        let bundled = bundled_of(bundled);

        let audit = audit_source(&entry, &nss, &bundled);
        let leaks = audit.leaks();
        let count = |c: SourceLeakChannel| leaks.iter().filter(|l| l.channel == c).count();

        prop_assert_eq!(count(SourceLeakChannel::EntryPreamble), usize::from(!entry.is_empty()));
        prop_assert_eq!(
            count(SourceLeakChannel::NamespacePreamble),
            nss.iter().filter(|n| !n.preamble.is_empty()).count()
        );
        prop_assert_eq!(
            count(SourceLeakChannel::BundledNamespace),
            bundled.iter().filter(|(_, s)| !s.is_empty()).count()
        );
    }

    /// Every leaking namespace is named, so the report is actionable.
    #[test]
    fn leaking_namespaces_are_attributed(
        preambles in prop::collection::vec("[a-z(); \n]{1,30}", 1..5),
        bundled in prop::collection::vec(("[a-z.]{1,8}", "[a-z(); \n]{1,30}"), 1..5),
    ) {
        let nss: Vec<_> = preambles.iter().enumerate()
            .map(|(i, p)| ns_of(&format!("ns{i}"), p)).collect();
        let bundled = bundled_of(bundled);

        let audit = audit_source("", &nss, &bundled);
        let leaks = audit.leaks();
        let named: Vec<&str> = leaks.iter().filter_map(|l| l.ns.as_deref()).collect();

        for n in &nss {
            prop_assert!(named.contains(&n.ns.as_ref()));
        }
        for (ns, _) in &bundled {
            prop_assert!(named.contains(&ns.as_ref()));
        }
        prop_assert!(leaks.iter().all(|l| l.bytes > 0));
    }

    /// Monotonic: adding source can never shrink the report. Guards against a
    /// gate that silently stops looking once it has found something.
    #[test]
    fn adding_source_never_shrinks_the_report(
        entry in "[a-z(); \n]{0,40}",
        preambles in prop::collection::vec("[a-z(); \n]{0,30}", 0..4),
        extra in "[a-z(); \n]{1,30}",
    ) {
        let nss: Vec<_> = preambles.iter().enumerate()
            .map(|(i, p)| ns_of(&format!("ns{i}"), p)).collect();

        let before = audit_source(&entry, &nss, &[]);
        let mut more = nss.clone();
        more.push(ns_of("extra", &extra));
        let after = audit_source(&entry, &more, &[]);

        prop_assert!(after.leaks().len() > before.leaks().len());
        prop_assert!(after.total_bytes() > before.total_bytes());
    }
}

proptest! {
    /// The report names the channel and the namespace, and quotes the byte
    /// count - it is the only thing telling a caller *what* to fix.
    #[test]
    fn rendered_leak_identifies_channel_ns_and_size(
        name in "[a-z.]{1,10}",
        src in "[a-z(); \n]{1,30}",
    ) {
        let nss = vec![ns_of(&name, &src)];
        let audit = audit_source("", &nss, &[]);
        let rendered = audit.leaks()[0].to_string();

        prop_assert!(rendered.contains(&name), "missing ns: {rendered}");
        prop_assert!(rendered.contains(&src.len().to_string()), "missing size: {rendered}");
        prop_assert!(rendered.contains("namespace preamble"), "missing channel: {rendered}");
    }

    /// The aggregate error lists every leak and the total, so a failed gate is
    /// actionable without re-running anything.
    #[test]
    fn rendered_error_lists_every_leak(
        entry in "[a-z(); \n]{1,20}",
        names in prop::collection::vec("[a-z.]{3,10}", 1..4),
    ) {
        let nss: Vec<_> = names.iter().map(|n| ns_of(n, "(ns x)\n")).collect();
        let audit = audit_source(&entry, &nss, &[]);
        let total = audit.total_bytes();
        let n = audit.leaks().len();
        let rendered = AotError::SourceEmbedded(audit).to_string();

        prop_assert!(rendered.contains(&n.to_string()));
        prop_assert!(rendered.contains(&total.to_string()));
        prop_assert!(rendered.contains("entry preamble"));
        for n in &names {
            prop_assert!(rendered.contains(n), "unlisted ns {n}: {rendered}");
        }
    }
}

#[test]
fn channel_names_are_distinct_and_stable() {
    let rendered: Vec<String> = [
        SourceLeakChannel::EntryPreamble,
        SourceLeakChannel::NamespacePreamble,
        SourceLeakChannel::BundledNamespace,
    ]
    .iter()
    .map(|c| c.to_string())
    .collect();

    assert_eq!(
        rendered,
        ["entry preamble", "namespace preamble", "bundled namespace"]
    );
}

#[test]
fn unattributed_leak_renders_without_a_namespace() {
    let audit = audit_source("(ns app)\n", &[], &[]);
    assert_eq!(audit.leaks()[0].to_string(), "entry preamble: 9 bytes");
}

#[test]
fn nothing_to_embed_is_clean() {
    assert!(audit_source("", &[], &[]).is_clean());
}

#[test]
fn compiled_namespace_still_leaks_its_preamble() {
    // The measured case: a namespace reported as successfully compiled ships
    // its defprotocol/defrecord/defmulti forms as readable text anyway.
    let nss = vec![ns_of(
        "helper",
        "(ns helper)\n(defrecord Weigher [bias] IWeigh (weigh [_ x] (+ (* x 31) bias)))\n",
    )];
    let audit = audit_source("", &nss, &[]);
    let leaks = audit.leaks();

    assert_eq!(leaks.len(), 1);
    assert_eq!(leaks[0].channel, SourceLeakChannel::NamespacePreamble);
    assert_eq!(leaks[0].ns.as_deref(), Some("helper"));
    assert!(leaks[0].bytes > 0);
}

proptest! {
    /// The policy layer is total and depends only on cleanliness: a clean audit
    /// passes under every policy, a dirty one is rejected only by the strict one.
    #[test]
    fn verdict_follows_cleanliness_and_policy(
        entry in "[a-z(); \n]{0,30}",
        preambles in prop::collection::vec("[a-z(); \n]{0,20}", 0..4),
    ) {
        let nss: Vec<_> = preambles.iter().enumerate()
            .map(|(i, p)| ns_of(&format!("ns{i}"), p)).collect();
        let audit = audit_source(&entry, &nss, &[]);

        let expected = |p| match (audit.is_clean(), p) {
            (true, _) => OpacityVerdict::Clean,
            (false, OpacityPolicy::Report) => OpacityVerdict::Tolerated,
            (false, OpacityPolicy::RequireFullyCompiled) => OpacityVerdict::Rejected,
        };

        for p in [OpacityPolicy::Report, OpacityPolicy::RequireFullyCompiled] {
            prop_assert_eq!(audit.verdict(p), expected(p));
        }
        // Only the strict policy may ever reject.
        prop_assert_ne!(audit.verdict(OpacityPolicy::Report), OpacityVerdict::Rejected);
    }
}

#[test]
fn default_policy_never_breaks_an_existing_build() {
    let dirty = audit_source("(ns app)\n", &[], &[]);
    assert_eq!(
        dirty.verdict(OpacityPolicy::default()),
        OpacityVerdict::Tolerated
    );
    assert_eq!(OpacityPolicy::default(), OpacityPolicy::Report);
}

// ── BundleAudit: the wasm side of the same policy ────────────────────────────

use cljrs_compiler::aot::{BundleAudit, Omission, OmissionKind};

fn omission_kind() -> impl Strategy<Value = OmissionKind> {
    prop_oneof![Just(OmissionKind::Namespace), Just(OmissionKind::EntryForm)]
}

fn bundle_audit() -> impl Strategy<Value = BundleAudit> {
    prop::collection::vec((omission_kind(), "[a-z.]{1,12}", "[a-z() ]{1,12}"), 0..6).prop_map(
        |rows| {
            BundleAudit::of(rows.into_iter().map(|(kind, ns, detail)| Omission {
                kind,
                ns: Some(ns),
                detail,
            }))
        },
    )
}

proptest! {
    /// Completeness is exactly "recorded nothing", the wasm analogue of
    /// SourceAudit's soundness property.
    #[test]
    fn complete_iff_no_omissions(a in bundle_audit()) {
        prop_assert_eq!(a.is_complete(), a.count() == 0);
        prop_assert_eq!(a.count(), a.omissions().len());
    }

    /// A complete module satisfies every policy; only the strict one can
    /// reject, and only an incomplete module.
    #[test]
    fn only_the_strict_policy_rejects(a in bundle_audit()) {
        prop_assert_ne!(a.verdict(OpacityPolicy::Report), OpacityVerdict::Rejected);
        let strict = a.verdict(OpacityPolicy::RequireFullyCompiled);
        if a.is_complete() {
            prop_assert_eq!(strict, OpacityVerdict::Clean);
        } else {
            prop_assert_eq!(strict, OpacityVerdict::Rejected);
        }
    }

    /// The report names every omission, since it is the only thing telling a
    /// caller which unit to move out of the compiled unit.
    #[test]
    fn the_report_lists_every_omission(a in bundle_audit()) {
        prop_assume!(!a.is_complete());
        let rendered = a.to_string();
        for o in a.omissions() {
            prop_assert!(rendered.contains(&o.detail), "{} missing from {}", o.detail, rendered);
        }
    }

    /// Adding an omission never removes one.
    #[test]
    fn recording_is_monotonic(a in bundle_audit(), ns in "[a-z]{1,8}") {
        let before = a.count();
        let grown = BundleAudit::of(a.omissions().iter().cloned().chain([Omission {
            kind: OmissionKind::Namespace,
            ns: Some(ns),
            detail: "x".to_string(),
        }]));
        prop_assert_eq!(grown.count(), before + 1);
    }
}
