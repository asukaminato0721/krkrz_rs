use krkrz_tjs::{Value, Vm, compile};
use serde::Deserialize;
#[derive(Deserialize)]
struct Case {
    name: String,
    source: String,
    expected: Value,
}
#[test]
fn original_random_generator_corpus() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/random_generator.json")).unwrap();
    for case in cases {
        let program = compile(&case.name, &case.source).unwrap();
        let value = Vm::default()
            .execute(&program, &mut (), &mut 1_000_000)
            .unwrap_or_else(|e| panic!("{}: {e:#}", case.name));
        assert_eq!(value, case.expected, "{}", case.name);
    }
}
#[test]
fn malformed_cursors_are_rejected_without_losing_existing_stream() {
    let program=compile("cursor", "var r=new Math.RandomGenerator(1),q=new Math.RandomGenerator(1),s=r.serialize();s.next=999999;s.left=99;try{r.randomize(s);}catch(e){return r.random32()==q.random32();}return false;").unwrap();
    assert_eq!(
        Vm::default()
            .execute(&program, &mut (), &mut 100_000)
            .unwrap(),
        Value::Integer(1)
    );
}
#[test]
fn unseeded_generators_replay_with_session_random_state() {
    let program = compile(
        "random",
        "var r=new Math.RandomGenerator();return r.random64();",
    )
    .unwrap();
    let run = || {
        Vm::default()
            .execute(&program, &mut (), &mut 100_000)
            .unwrap()
    };
    assert_eq!(run(), run());
}
