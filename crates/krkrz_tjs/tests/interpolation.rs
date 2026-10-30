use krkrz_tjs::{Value, Vm, compile};

#[test]
fn interpolation_allows_whitespace_after_the_at_sign() {
    let program = compile(
        "interpolation",
        "var n=8; return [@  \"bits |= ${n}\", @\n\t'value=${n+1}'].join('|');",
    )
    .unwrap();
    assert_eq!(
        Vm::default().execute(&program, &mut (), &mut 1000).unwrap(),
        Value::string("bits |= 8|value=9")
    );
}
