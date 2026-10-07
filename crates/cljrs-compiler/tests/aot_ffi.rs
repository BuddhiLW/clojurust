//! `clojure.rust.ffi` through `cljrs compile`: the namespace resolves at
//! compile time and the produced binary calls into a system C library.

mod common;

/// libm's `cos` and `pow`, and an `:ffi/error` caught as data, from an AOT
/// binary. Linux only: the soname is glibc's.
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
#[test]
fn aot_binary_calls_libm_through_clojure_rust_ffi() {
    let dir = tempfile::tempdir().expect("temp dir");
    let src = dir.path().join("ffi_main.cljrs");
    let bin = dir.path().join("ffi_main_bin");
    std::fs::write(
        &src,
        r#"(ns ffi-main (:require [clojure.rust.ffi :as ffi]))
(def libm (ffi/open "libm.so.6"))
(println ((ffi/function libm "cos" [:double] :double) 0.0))
(println (ffi/call libm "pow" [:double :double] :double 2.0 10.0))
(println (:ffi/error (try (ffi/sym libm "no_such_symbol") (catch :default e (ex-data e)))))
(ffi/close libm)
"#,
    )
    .unwrap();

    let result = std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn({
            let (src, bin) = (src.clone(), bin.clone());
            move || cljrs_compiler::aot::compile_file(&src, &bin, &common::session(vec![]))
        })
        .unwrap()
        .join()
        .unwrap();
    result.unwrap_or_else(|e| panic!("compilation failed: {e:?}"));

    let out = std::process::Command::new(&bin)
        .output()
        .expect("run binary");
    assert!(
        out.status.success(),
        "binary failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8(out.stdout).unwrap().trim(),
        "1.0\n1024.0\n:symbol"
    );
}
