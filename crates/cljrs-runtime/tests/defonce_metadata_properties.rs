//! Property tests for `defonce`'s name extraction: `def` and `defonce` agree
//! over every name and metadata shape.

use std::sync::Arc;

use cljrs_reader::Parser;
use cljrs_runtime::env::env::{Env, GlobalEnv};
use cljrs_value::Value;
use proptest::prelude::*;

fn make_env() -> (Arc<GlobalEnv>, Env) {
    let globals = cljrs_runtime::interp::standard_env(None, None, None);
    let env = Env::new(globals.clone(), "user");
    (globals, env)
}

fn eval_fresh(src: &str) -> Result<Value, String> {
    let (_, mut env) = make_env();
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().map_err(|e| format!("{e:?}"))?;
    let mut result = Value::Nil;
    for form in forms {
        result =
            cljrs_runtime::interp::eval::eval(&form, &mut env).map_err(|e| format!("{e:?}"))?;
    }
    Ok(result)
}

/// An identifier that cannot collide with `clojure.core`.
fn var_name() -> impl Strategy<Value = String> {
    "[a-z]{1,6}".prop_map(|s| format!("pt-{s}"))
}

/// Metadata spellings `def` accepts on a name, including none.
fn meta_prefix() -> impl Strategy<Value = String> {
    prop_oneof![
        Just(String::new()),
        Just("^:private ".to_string()),
        Just("^:dynamic ".to_string()),
        Just("^{:doc \"d\"} ".to_string()),
        Just("^:private ^{:doc \"d\"} ".to_string()),
        Just("^{:a 1 :b 2} ".to_string()),
    ]
}

proptest! {
    /// `def` and `defonce` extract the same name from the same form.
    #[test]
    fn defonce_agrees_with_def_on_the_name(name in var_name(), m in meta_prefix()) {
        let by_def = eval_fresh(&format!("(def {m}{name} 42) {name}"))
            .expect("def accepts a metadata-carrying symbol");
        let by_defonce = eval_fresh(&format!("(defonce {m}{name} 42) {name}"))
            .expect("defonce must accept the same symbol def does");
        prop_assert_eq!(format!("{by_def:?}"), format!("{by_defonce:?}"));
    }

    /// Metadata reaches the var, not just parses.
    #[test]
    fn metadata_reaches_the_var(name in var_name()) {
        let result = eval_fresh(&format!(
            "(defonce ^:private ^{{:doc \"d\"}} {name} 1) \
             [(:private (meta (var {name}))) (:doc (meta (var {name})))]"
        ))
        .expect("defonce with metadata should evaluate");
        let rendered = format!("{result:?}");
        prop_assert!(rendered.contains("true"), "private flag missing: {}", rendered);
        prop_assert!(rendered.contains('d'), "doc missing: {}", rendered);
    }

    /// A second `defonce` of a bound var is a no-op, for every metadata shape.
    #[test]
    fn defonce_never_rebinds(name in var_name(), m in meta_prefix()) {
        let result = eval_fresh(&format!(
            "(defonce {m}{name} 1) (defonce {m}{name} 2) {name}"
        ))
        .expect("a repeated defonce should evaluate");
        prop_assert_eq!(format!("{result:?}"), format!("{:?}", Value::Long(1)));
    }

    /// Extraction is total over the metadata shapes `def` accepts.
    #[test]
    fn no_metadata_shape_is_rejected(name in var_name(), m in meta_prefix()) {
        let outcome = eval_fresh(&format!("(defonce {m}{name} 1)"));
        prop_assert!(outcome.is_ok(), "rejected {m}{name}: {:?}", outcome.unwrap_err());
    }
}
