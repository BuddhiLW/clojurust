# Changelog

- Reader conditionals select the first matching feature in textual order. `:cljrs` is the platform feature; `:rust` is an accepted alias. Neither `:clj` nor `:cljs` matches.

- Project native libraries now export an ABI fingerprint via `cljrs_interop::export_init!`; mismatches refuse startup before init, while legacy libraries warn (or refuse with `CLJRS_NATIVE_STRICT=1`). `cljrs build-native` reports the built fingerprint.
