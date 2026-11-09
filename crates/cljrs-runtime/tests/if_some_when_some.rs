use cljrs_reader::Parser;
use cljrs_runtime::{ExecutionMode, Runtime};
use cljrs_value::Value;

/// `if-some` / `when-some` bind on any non-nil value, including `false`.
#[test]
fn if_some_and_when_some_bind_non_nil() {
    let runtime = Runtime::builder()
        .execution_mode(ExecutionMode::TreeWalk)
        .build()
        .expect("bootstrap");
    let mut env = runtime.env("user");
    let src = "[(if-some [x false] [:got x] :nil)
                (if-some [x nil] :got :nil)
                (when-some [x 0] (inc x))
                (when-some [x nil] :never)]";
    let forms = Parser::new(src.to_string(), "<test>".to_string())
        .parse_all()
        .expect("parse");
    let mut result = Value::Nil;
    for form in &forms {
        result = cljrs_runtime::interp::eval::eval(form, &mut env).expect("eval");
    }
    assert_eq!(result.to_string(), "[[:got false] :nil 1 nil]");
}
