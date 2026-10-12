//! `{:keys [...]}` accepts keywords as well as symbols, as in Clojure:
//! `{:keys [:a ::b]}` binds `a` and `b`, the auto-resolved `::b` against
//! the current namespace. malli.core's `:multi` schema destructures
//! `{:keys [::default]}`.

use cljrs_reader::Parser;
use cljrs_runtime::env::env::Env;
use cljrs_value::Value;

fn eval_in(mode: cljrs_runtime::ExecutionMode, src: &str) -> Value {
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(mode)
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

const SRC: &str = "(ns kk.a)
     (defn f [{:keys [::default :plain :x/q]}] [default plain q])
     (= [1 2 3] (f {::default 1 :plain 2 :x/q 3})
        (let [{:keys [::default]} {::default 1}] [default 2 3]))";

#[test]
fn keys_accepts_keywords_tree_walk() {
    assert_eq!(
        eval_in(cljrs_runtime::ExecutionMode::TreeWalk, SRC),
        Value::Bool(true)
    );
}

#[test]
fn keys_accepts_keywords_default_mode() {
    assert_eq!(
        eval_in(cljrs_runtime::ExecutionMode::default(), SRC),
        Value::Bool(true)
    );
}
