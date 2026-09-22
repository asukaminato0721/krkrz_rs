use krkrz_tjs::{Value, Vm, VmAbort, compile};

fn run(input: Vec<u16>, source: &str, mut budget: u64) -> anyhow::Result<Value> {
    let mut vm = Vm::default();
    vm.globals.insert("input".into(), Value::String(input));
    vm.execute(
        &compile("escape-test", source)?,
        &mut (),
        &mut budget,
    )
}

#[test]
fn escapes_control_characters_quotes_and_backslashes_without_adding_quotes() {
    let input = vec![7, 8, 12, 10, 13, 9, 11, 92, 39, 34];
    assert_eq!(
        run(input, "return input.escape();", 1000).unwrap(),
        Value::string(r#"\a\b\f\n\r\t\v\\\'\""#)
    );
    assert_eq!(
        run(vec![], "return input.escape();", 1000).unwrap(),
        Value::string("")
    );
}

#[test]
fn hex_digit_runs_are_unambiguous_and_round_trip_through_eval() {
    let input = vec![
        1,
        b'A' as u16,
        b'f' as u16,
        b'9' as u16,
        b'G' as u16,
        31,
        b'0' as u16,
    ];
    assert_eq!(
        run(input.clone(), "return input.escape();", 1000).unwrap(),
        Value::string(r"\x01\x41\x66\x39G\x1f\x30")
    );
    assert_eq!(
        run(
            input,
            r#"return eval('"'+input.escape()+'"')===input;"#,
            1000
        )
        .unwrap(),
        Value::Integer(1)
    );
}

#[test]
fn preserves_utf16_units_stops_at_nul_and_ignores_extra_arguments() {
    let input = vec![0x65e5, 0xd800, 0xdc00, 0xdfff, 0, 65];
    assert_eq!(
        run(input.clone(), "return input.escape(123);", 1000).unwrap(),
        Value::String(input[..4].to_vec())
    );
    assert_eq!(
        run(vec![10], "var f=input.escape;return f();", 1000).unwrap(),
        Value::string(r"\n")
    );
}

#[test]
fn escaping_obeys_execution_budget_and_skips_unused_results() {
    let input = vec![65; 1000];
    let error = run(
        input.clone(),
        "try{return input.escape();}catch(e){return 7;}",
        100,
    )
    .unwrap_err();
    assert!(error.downcast_ref::<VmAbort>().is_some(), "{error:#}");
    assert_eq!(
        run(input, "input.escape();return 1;", 100).unwrap(),
        Value::Integer(1)
    );
}
