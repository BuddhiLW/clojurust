use cljrs_reader::Parser;
use cljrs_runtime::{ExecutionMode, Runtime};
use cljrs_value::Value;

/// `count` and `apply` must accept an object-array (malli's entry parser builds
/// `(object-array n)` buffers and then counts and spreads them).
#[test]
fn count_and_apply_accept_object_array() {
    let runtime = Runtime::builder()
        .execution_mode(ExecutionMode::TreeWalk)
        .build()
        .expect("bootstrap");
    let mut env = runtime.env("user");
    let src = "(let [a (object-array [:a 1 :b 2])]
                 [(count a) (apply vector a) (apply array-map a)])";
    let forms = Parser::new(src.to_string(), "<test>".to_string())
        .parse_all()
        .expect("parse");
    let mut result = Value::Nil;
    for form in &forms {
        result = cljrs_runtime::interp::eval::eval(form, &mut env).expect("eval");
    }
    assert_eq!(result.to_string(), "[4 [:a 1 :b 2] {:a 1, :b 2}]");
}
