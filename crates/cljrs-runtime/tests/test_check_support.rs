//! Host features `clojure.test.check` needs to load and run on cljrs
//! (the dependency `malli.generator` pulls in). Each test pins one failure
//! seen while requiring test.check 1.1.1 from source.

use cljrs_reader::Parser;
use cljrs_runtime::env::env::Env;
use cljrs_value::Value;

fn eval_pr(src: &str) -> String {
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
        .build()
        .expect("runtime")
        .into_globals();
    let mut env = Env::new(globals, "user");
    let wrapped = format!("(pr-str (do {src}))");
    let mut parser = Parser::new(wrapped, "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(&form, &mut env).expect("eval error");
    }
    match result {
        Value::Str(s) => s.get().as_str().to_string(),
        other => panic!("expected a string, got {other:?}"),
    }
}

/// clojure.test.check.random / generators use these JVM numeric statics.
#[test]
fn jvm_numeric_statics() {
    assert_eq!(
        eval_pr(
            "[(Long/bitCount 255) Long/MIN_VALUE (Long/reverse 1) (Math/scalb 1.5 3)
              (Math/getExponent 8.0) (Character/isDigit \\3) (Double/isNaN Double/NaN)
              (= Double/POSITIVE_INFINITY (- Double/NEGATIVE_INFINITY)) (> Double/MAX_VALUE 1e308)]"
        ),
        "[8 -9223372036854775808 -9223372036854775808 12.0 3 true true true true]"
    );
}
