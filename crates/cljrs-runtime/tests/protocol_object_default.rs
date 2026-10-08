//! `extend-type Object` catches every non-nil value and `extend-type default`
//! catches every value, when no exact type impl exists (JVM / ClojureScript
//! semantics). malli.core extends its protocols to `Object` for plain schemas.

use cljrs_reader::Parser;
use cljrs_runtime::env::env::Env;
use cljrs_value::Value;

fn eval_src(src: &str) -> Value {
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
        .build()
        .expect("runtime")
        .into_globals();
    let mut env = Env::new(globals, "user");
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(&form, &mut env).expect("eval error");
    }
    result
}

#[test]
fn object_extension_catches_non_nil_values() {
    let v = eval_src(
        "(defprotocol P (f [x]))
         (extend-type Object P (f [_] :obj))
         (extend-type Long P (f [_] :long))
         (= [:obj :obj :long] [(f (reify)) (f \"s\") (f 1)])",
    );
    assert_eq!(v, Value::Bool(true));
}

#[test]
fn default_extension_catches_everything_including_nil() {
    let v = eval_src(
        "(defprotocol Q (g [x]))
         (extend-type default Q (g [_] :dflt))
         (= [:dflt :dflt :dflt] [(g (reify)) (g 1) (g nil)])",
    );
    assert_eq!(v, Value::Bool(true));
}
