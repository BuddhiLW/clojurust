//! Two extension crates must not export the same C-ABI init symbol.
//!
//! This small identity check complements the AOT end-to-end test that links the
//! default Base64 extension beside a user crate's `:rust :init` hook.

/// A shared `#[no_mangle] cljrs_init` can either fail with a duplicate-symbol
/// error or silently resolve both Rust paths to one definition. Which result
/// occurs depends on whether another reference pulls both defining archive
/// members into the link.
///
/// Naming each export after its crate is what makes that unrepresentable, and
/// this pins it. Measured before the rename: the two addresses were equal.
#[test]
fn extension_init_symbols_are_unique_per_crate() {
    let base64 = cljrs_base64::cljrs_init_cljrs_base64 as *const () as usize;
    let blake3 = cljrs_blake3::cljrs_init_cljrs_blake3 as *const () as usize;
    assert_ne!(
        base64, blake3,
        "two extension crates resolved their init hooks to one address"
    );
}
