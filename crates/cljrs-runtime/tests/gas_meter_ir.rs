use cljrs_reader::Parser;
use cljrs_runtime::env::error::EvalError;

fn forms(src: &str) -> Vec<cljrs_reader::Form> {
    Parser::new(src.to_owned(), "<gas-ir-test>".to_owned())
        .parse_all()
        .expect("parse")
}

fn env() -> cljrs_runtime::tiered::Env {
    cljrs_runtime::tiered::Env::new(cljrs_runtime::tiered::standard_env(), "user")
}

#[test]
fn ir_function_exhausts_meter() {
    cljrs_runtime::tiered::force_eager_lowering();
    let mut env = env();
    for form in forms("(defn spin [n] (if (= n 0) 0 (spin (dec n))))") {
        cljrs_runtime::tiered::eval(&form, &mut env).expect("defn");
    }
    let call = forms("(spin 100000)").remove(0);
    assert!(matches!(
        cljrs_runtime::tiered::eval_with_gas(&call, &mut env, 100),
        Err(EvalError::GasExhausted)
    ));
}

#[test]
fn nested_meter_charges_outer_budget() {
    let mut env = env();
    let outer = cljrs_runtime::env::gas::GasMeter::new(20);
    let _outer_guard = cljrs_runtime::env::gas::GasGuard::install(outer.clone());
    let form = forms("1").remove(0);
    cljrs_runtime::tiered::eval_with_gas(&form, &mut env, 10).expect("inner eval");
    assert!(
        outer.remaining() < 20,
        "inner eval did not charge outer meter"
    );
}
