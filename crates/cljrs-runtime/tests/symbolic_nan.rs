//! Regression coverage for symbolic NaN macro expansion (issue #280).

use cljrs_reader::Parser;
use cljrs_runtime::env::env::Env;
use cljrs_value::Value;

#[test]
fn symbolic_nan_evaluates_without_hanging() {
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
        .eager_clojure_test(true)
        .build()
        .expect("runtime")
        .into_globals();
    let mut env = Env::new(globals, "user");
    let mut parser = Parser::new("(NaN? ##NaN)".to_string(), "<test>".to_string());
    let form = parser
        .parse_all()
        .expect("parse error")
        .into_iter()
        .next()
        .expect("missing form");

    assert_eq!(
        cljrs_runtime::interp::eval::eval(&form, &mut env).expect("eval error"),
        Value::Bool(true)
    );
}
