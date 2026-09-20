use krkrz_runtime::Session;
use krkrz_tjs::{Value, VmAbort};
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    name: String,
    source: String,
    expected: Value,
}

#[test]
fn original_missing_format_and_math_corpus() {
    for fixture in [
        include_str!("fixtures/missing.json"),
        include_str!("fixtures/sprintf.json"),
        include_str!("fixtures/math.json"),
        include_str!("fixtures/assign_struct.json"),
    ] {
        for case in serde_json::from_str::<Vec<Case>>(fixture).unwrap() {
            let project = tempfile::tempdir().unwrap();
            let saves = tempfile::tempdir().unwrap();
            std::fs::write(project.path().join("case.tjs"), case.source).unwrap();
            let mut session =
                Session::open(project.path(), Some(saves.path()), false, 100_000).unwrap();
            assert_eq!(
                session
                    .execute_storage("case.tjs")
                    .unwrap_or_else(|e| panic!("{}: {e:#}", case.name)),
                case.expected,
                "{}",
                case.name
            );
        }
    }
}

#[test]
fn missing_budget_abort_releases_reentrancy_guard() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), false, 100_000).unwrap();
    std::fs::write(
        project.path().join("case.tjs"),
        r#"
class C {var loop=true; function missing(set,name,value) {while(loop){} *value=17;return true;}}
global.o=new C();Scripts.setCallMissing(o);
"#,
    )
    .unwrap();
    session.execute_storage("case.tjs").unwrap();
    session.budget = 200;
    let error = session.evaluate("o.x").unwrap_err();
    assert!(error.downcast_ref::<VmAbort>().is_some(), "{error:#}");
    session.budget = 10_000;
    session.evaluate("o.loop=false").unwrap();
    assert_eq!(session.evaluate("o.x").unwrap(), Value::Integer(17));
}

#[test]
fn math_random_state_restores_the_sequence_and_discarded_calls_do_not_advance_it() {
    let mut vm = krkrz_tjs::Vm::default();
    let state = vm.random_state();
    let discarded = krkrz_tjs::compile("discard", "Math.random();").unwrap();
    vm.execute(&discarded, &mut (), &mut 1000).unwrap();
    assert_eq!(state, vm.random_state());
    let next = krkrz_tjs::compile("random", "return Math.random();").unwrap();
    let first = vm.execute(&next, &mut (), &mut 1000).unwrap();
    let second = vm.execute(&next, &mut (), &mut 1000).unwrap();
    assert_ne!(first, second);
    vm.set_random_state(state);
    assert_eq!(first, vm.execute(&next, &mut (), &mut 1000).unwrap());
    assert_eq!(second, vm.execute(&next, &mut (), &mut 1000).unwrap());
}
