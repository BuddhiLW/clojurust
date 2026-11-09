//! Exercise the native ABI directly; no background JIT promotion is required.
use cljrs_compiler::rt_abi::rt_identical;
use cljrs_value::Value;

fn same(a: &Value, b: &Value) -> bool {
    // Both pointers are live for the call; rt_identical returns an interned bool.
    unsafe { matches!(&*rt_identical(a, b), Value::Bool(true)) }
}

#[test]
fn native_identity_compares_values_not_argument_boxes() {
    for value in [
        Value::Nil,
        Value::Bool(true),
        Value::Bool(false),
        Value::Long(7),
        Value::Char('c'),
    ] {
        let other_box = value.clone();
        assert!(same(&value, &other_box), "{value:?}");
    }
    assert!(!same(&Value::Bool(true), &Value::Bool(false)));
    assert!(!same(&Value::Long(1), &Value::Bool(true)));
    let s = Value::string("same");
    assert!(same(&s, &s.clone()));
    assert!(!same(&s, &Value::string("same")));
}
