use cljrs_reader::Parser;
use cljrs_runtime::{ExecutionMode, Runtime};
use cljrs_value::Value;

/// A reify/deftype method named like its protocol function must dispatch to
/// the protocol function when called inside its own body (malli registries
/// delegate `-schema` to another registry this way), not recurse into itself.
#[test]
fn method_body_calls_protocol_fn_of_same_name_on_other_instance() {
    let runtime = Runtime::builder()
        .execution_mode(ExecutionMode::TreeWalk)
        .build()
        .expect("bootstrap");
    let mut env = runtime.env("user");
    let src = "(defprotocol P (-s [this t]))
               (def inner (reify P (-s [_ t] [:inner t])))
               (defn mk [a] (reify P (-s [_ t] (-s a t))))
               (-s (mk inner) 2)";
    let forms = Parser::new(src.to_string(), "<test>".to_string())
        .parse_all()
        .expect("parse");
    let mut result = Value::Nil;
    for form in &forms {
        result = cljrs_runtime::interp::eval::eval(form, &mut env).expect("eval");
    }
    assert_eq!(result.to_string(), "[:inner 2]");
}
