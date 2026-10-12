use cljrs_reader::Parser;
use cljrs_runtime::{ExecutionMode, Runtime};
use cljrs_value::Value;

#[test]
fn top_level_reader_conditionals_follow_platform_order() {
    let runtime = Runtime::builder()
        .execution_mode(ExecutionMode::TreeWalk)
        .build()
        .expect("bootstrap");
    let mut env = runtime.env("user");
    for (src, expected) in [
        (
            "#?(:cljrs :c :clj :j)",
            Value::keyword(cljrs_value::Keyword::simple("c")),
        ),
        (
            "#?(:clj :j :rust :r)",
            Value::keyword(cljrs_value::Keyword::simple("r")),
        ),
        (
            "#?(:clj :j :default :d)",
            Value::keyword(cljrs_value::Keyword::simple("d")),
        ),
        ("#?(:clj :j :cljs :s)", Value::Nil),
    ] {
        let form = Parser::new(src.to_string(), "<test>".to_string())
            .parse_one()
            .expect("parse")
            .expect("form");
        assert_eq!(
            cljrs_runtime::interp::eval::eval(&form, &mut env).expect("eval"),
            expected,
            "{src}"
        );
    }
}
