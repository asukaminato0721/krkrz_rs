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
fn storage_block_functions_exist_even_in_unexecuted_branches() {
    let program = compile(
        "block-declaration",
        "var result=hidden();if(false) {function hidden(){return 9;}} return result;",
    )
    .unwrap();
    assert_eq!(
        Vm::default().execute(&program, &mut (), &mut 1000).unwrap(),
        Value::Integer(9)
    );
}

#[test]
fn nested_storage_functions_survive_blocks_but_variables_do_not() {
    let program = compile(
        "nested-block-declarations",
        "var result=soundUpdate();{{var local=4;function soundUpdate(){return 12;}}}return result+':'+typeof global.local;",
    )
    .unwrap();
    assert_eq!(
        Vm::default().execute(&program, &mut (), &mut 1000).unwrap(),
        Value::string("12:undefined")
    );
}

#[test]
fn storage_block_declarations_keep_their_definition_context() {
    let mut vm = Vm::default();
    let context = vm.new_dictionary().unwrap();
    vm.set_member(&context, &Value::string("value"), Value::Integer(4))
        .unwrap();
    vm.execute_in_context(
        &compile(
            "context-block-declaration",
            "{{function soundUpdate(){return this.value;}}}",
        )
        .unwrap(),
        &context,
        &mut (),
        &mut 1000,
    )
    .unwrap();
    let function = vm
        .get_member(&context, &Value::string("soundUpdate"), false)
        .unwrap();
    let other = vm.new_dictionary().unwrap();
    vm.set_member(&other, &Value::string("value"), Value::Integer(99))
        .unwrap();
    assert_eq!(
        vm.call_function(&function, &other, &[], &mut (), &mut 1000)
            .unwrap(),
        Value::Integer(4)
    );
    assert!(!vm.globals.contains_key("soundUpdate"));
}

#[test]
fn storage_block_classes_and_properties_exist_before_execution() {
    let program = compile(
        "block-class-property",
        "return (new Hidden()).value+answer; if(false){class Hidden{var value=4;} property answer{getter{return 3;}}}",
    )
    .unwrap();
    assert_eq!(
        Vm::default().execute(&program, &mut (), &mut 1000).unwrap(),
        Value::Integer(7)
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
