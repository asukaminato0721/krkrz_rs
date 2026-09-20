use krkrz_tjs::{Value, Vm, VmAbort, compile};
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    name: String,
    source: String,
    expected: Value,
}
#[test]
fn original_startup_builtin_corpus() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/startup_builtins.json")).unwrap();
    for case in cases {
        let program = compile(&case.name, &case.source).unwrap();
        let value = Vm::default().execute(&program, &mut (), &mut 100_000).unwrap_or_else(|e| panic!("{}: {e:#}", case.name));
        assert_eq!(value, case.expected, "{}", case.name);
    }
}
#[test]
fn community_reference_corpus() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/semantics.json")).unwrap();
    for case in cases {
        let program =
            compile(&case.name, &case.source).unwrap_or_else(|e| panic!("{}: {e:#}", case.name));
        let value = Vm::default()
            .execute(&program, &mut (), &mut 10_000)
            .unwrap_or_else(|e| panic!("{}: {e:#}", case.name));
        assert_eq!(value, case.expected, "{}", case.name);
    }
}
#[test]
fn script_catch_cannot_hide_budget_or_missing_native_implementation() {
    for source in [
        "try { while(1) {} } catch { return 42; }",
        "function f(){while(1){}} try { f(); } catch { return 42; }",
        "try { Unknown(); } catch { return 42; }",
    ] {
        let mut vm = Vm::default();
        vm.register_native("Unknown").unwrap();
        let error = vm
            .execute(&compile("fatal", source).unwrap(), &mut (), &mut 100)
            .unwrap_err();
        assert!(error.downcast_ref::<VmAbort>().is_some(), "{error:#}");
    }
}
#[test]
fn thrown_object_preserves_identity_and_handlers_unwind_with_scopes() {
    let source = "var o=%['message'=>'original']; try{with(o){try{throw o;}catch(e){e.message='changed';throw e;}}}catch(e){return (e===o) && (e.message=='changed');}";
    let result = Vm::default()
        .execute(&compile("unwind", source).unwrap(), &mut (), &mut 1000)
        .unwrap();
    assert_eq!(result, Value::Integer(1));
}

#[test]
fn adjacent_strings_require_whitespace_and_matching_delimiters() {
    for source in ["return 'a' /*comment*/ 'b';", "return 'a' \"b\";"] {
        assert!(compile("adjacent", source).is_err(), "{source}");
    }
}

#[test]
fn invalid_builtin_constructor_is_rejected_without_crashing() {
    // Constructing from an invalidated Array/RegExp class crashes the isolated
    // target executable. This safety regression is not an oracle agreement case.
    let source = "var a=new RegExp('x');invalidate RegExp;try{new RegExp('y');}catch(e){return isvalid a;}return 0;";
    let value = Vm::default()
        .execute(
            &compile("invalid native class", source).unwrap(),
            &mut (),
            &mut 1000,
        )
        .unwrap();
    assert_eq!(value, Value::Integer(1));
}
