# Changelog

- Project native libraries now export an ABI fingerprint via `cljrs_interop::export_init!`; mismatches refuse startup before init, while legacy libraries warn (or refuse with `CLJRS_NATIVE_STRICT=1`). `cljrs build-native` reports the built fingerprint.
