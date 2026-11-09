use cljrs_reader::Parser;
use cljrs_runtime::{ExecutionMode, Runtime};
use cljrs_value::Value;

#[test]
fn compiler_flags_are_dynamic_and_settable_at_top_level() {
    let runtime = Runtime::builder()
        .execution_mode(ExecutionMode::TreeWalk)
        .build()
        .expect("bootstrap");
    let mut env = runtime.env("user");
    let src = "(set! *unchecked-math* true) (set! *warn-on-reflection* true) [(deref #'*unchecked-math*) (deref #'*warn-on-reflection*)]";
    let forms = Parser::new(src.to_string(), "<test>".to_string())
        .parse_all()
        .expect("parse");
    let mut result = Value::Nil;
    for form in &forms {
        result = cljrs_runtime::interp::eval::eval(form, &mut env).expect("eval");
    }
    assert_eq!(result.to_string(), "[true true]");
}
