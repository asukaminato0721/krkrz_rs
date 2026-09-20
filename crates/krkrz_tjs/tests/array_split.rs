use krkrz_tjs::{Value, Vm, VmAbort, compile};
#[test]
fn original_array_split_corpus() {
    #[derive(serde::Deserialize)]
    struct Case { name: String, source: String, expected: Value }
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/array_split.json")).unwrap();
    for case in cases {
        let mut vm = Vm::default();
        let program = compile(&case.name, &case.source).unwrap();
        assert_eq!(vm.execute(&program, &mut (), &mut 100_000).unwrap_or_else(|e| panic!("{}: {e:#}", case.name)), case.expected, "{}", case.name);
    }
}
#[test]
fn split_work_is_charged_and_cannot_be_hidden_by_script_catch() {
    let source = "var s='x'.repeat(1000),a=[];try{a.split('x',s);}catch(e){return 99;}";
    let error = Vm::default().execute(&compile("budget", source).unwrap(), &mut (), &mut 1_500).unwrap_err();
    assert!(error.downcast_ref::<VmAbort>().is_some(), "{error:#}");
}
