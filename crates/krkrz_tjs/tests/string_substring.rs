use krkrz_tjs::{Value, Vm, compile};

fn run(source: &str) -> Value {
    Vm::default()
        .execute(&compile("substring", source).unwrap(), &mut (), &mut 10_000)
        .unwrap()
}

#[test]
fn substring_and_substr_use_the_same_start_and_count() {
    for method in ["substring", "substr"] {
        for (args, expected) in [
            ("2,3", "cde"),
            ("3,1", "d"),
            ("2,99", "cdef"),
            ("2", "cdef"),
            ("-1,3", ""),
            ("6,1", ""),
            ("99", ""),
            ("2,-1", ""),
            ("2,0", ""),
            ("2,void", ""),
            ("4294967297,2", "bc"),
        ] {
            assert_eq!(
                run(&format!("return 'abcdef'.{method}({args});")),
                Value::string(expected),
                "{method}({args})"
            );
        }
        assert_eq!(
            run(&format!(
                "var s='image:\tbackground';return s.{method}(6,s.indexOf('\\t')-5).trim();"
            )),
            Value::string("")
        );
    }
}

#[test]
fn substring_counts_utf16_units_and_checks_argument_count() {
    let mut vm = Vm::default();
    vm.globals
        .insert("input".into(), Value::String(vec![65, 0xd83d, 0xde00, 66]));
    assert_eq!(
        vm.execute(
            &compile("UTF-16 substring", "return input.substring(1,1);").unwrap(),
            &mut (),
            &mut 1000
        )
        .unwrap(),
        Value::String(vec![0xd83d])
    );
    for method in ["substring", "substr"] {
        for args in ["", "0,1,2"] {
            assert_eq!(
                run(&format!(
                    "try{{'abc'.{method}({args});return 0;}}catch(e){{return 1;}}"
                )),
                Value::Integer(1)
            );
        }
    }
}
