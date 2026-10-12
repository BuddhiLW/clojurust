use cljrs_reader::Parser;
use cljrs_runtime::{ExecutionMode, Runtime};
use cljrs_value::Value;

/// malli's predicate registry names `indexed?`, `uri?` and `inst?`; each must
/// resolve in clojure.core.
#[test]
fn indexed_uri_inst_predicates_resolve() {
    let runtime = Runtime::builder()
        .execution_mode(ExecutionMode::TreeWalk)
        .build()
        .expect("bootstrap");
    let mut env = runtime.env("user");
    let src = "[(indexed? [1]) (indexed? '(1)) (uri? \"x\") (inst? 1)]";
    let forms = Parser::new(src.to_string(), "<test>".to_string())
        .parse_all()
        .expect("parse");
    let mut result = Value::Nil;
    for form in &forms {
        result = cljrs_runtime::interp::eval::eval(form, &mut env).expect("eval");
    }
    assert_eq!(result.to_string(), "[true false false false]");
}
