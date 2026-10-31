use krkrz_tjs::{Value, Vm, compile};

#[test]
fn storage_functions_are_available_before_their_declaration() {
    let source = r#"
var result = forward(3);
try { if (result != 7) throw 'wrong result'; } catch { result = -100; }
return result;
function forward(n) { return next(n) + 1; }
function next(n) { return n * 2; }
"#;
    let program = compile("forward-declarations", source).unwrap();
    assert_eq!(
        Vm::default().execute(&program, &mut (), &mut 1000).unwrap(),
        Value::Integer(7)
    );
}

#[test]
fn block_functions_keep_their_scope() {
    let program = compile(
        "block-declaration",
        "if(false) {function hidden(){return 1;}} return typeof global.hidden;",
    )
    .unwrap();
    assert_eq!(
        Vm::default().execute(&program, &mut (), &mut 1000).unwrap(),
        Value::string("undefined")
    );
}

#[test]
fn storage_classes_and_properties_are_available_before_their_declaration() {
    let program = compile(
        "forward-class",
        "C.counter=4;var c=new C();return c.value+answer; class C {var value=C.counter;} property answer {getter{return 3;}}",
    )
    .unwrap();
    assert_eq!(
        Vm::default().execute(&program, &mut (), &mut 1000).unwrap(),
        Value::Integer(7)
    );
}

#[test]
fn expression_mode_accepts_trailing_semicolons() {
    let program = krkrz_tjs::compile_expression("envinit", "%[value:7]; /* end */;").unwrap();
    let mut vm = Vm::default();
    let result = vm.execute(&program, &mut (), &mut 1000).unwrap();
    assert_eq!(
        vm.get_member(&result, &Value::string("value"), false)
            .unwrap(),
        Value::Integer(7)
    );
    assert!(krkrz_tjs::compile_expression("extra", "1;2").is_err());
}
