use krkrz_runtime::Session;
use krkrz_tjs::Value;

#[test]
fn shell_execute_forwards_native_arguments_and_returns_boolean_status() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    let mut s = Session::open(project.path(), Some(saves.path()), None, 100_000).unwrap();
    assert!(s.evaluate("System.shellExecute()").is_err());
    // A deterministic/headless session must never launch desktop applications.
    assert_eq!(
        s.evaluate("System.shellExecute('https://example.invalid/')")
            .unwrap(),
        Value::Integer(0)
    );
    let mut requests = [
        ("https://example.invalid/?a=1&b=2", "", Ok(true)),
        ("説明.txt", "", Ok(false)),
        ("program.exe", "\"argument with spaces\" /flag", Ok(true)),
        ("failure", "", Err(anyhow::anyhow!("opener unavailable"))),
    ]
    .into_iter();
    s.services
        .set_shell_execute_handler(move |target, parameters| {
            let (expected_target, expected_parameters, result) = requests.next().unwrap();
            assert_eq!((target, parameters), (expected_target, expected_parameters));
            result
        });
    for (expression, expected) in [
        ("System.shellExecute('https://example.invalid/?a=1&b=2')", 1),
        ("System.shellExecute('説明.txt',void)", 0),
        (
            "System.shellExecute('program.exe','\"argument with spaces\" /flag')",
            1,
        ),
        ("System.shellExecute('failure')", 0),
    ] {
        assert_eq!(s.evaluate(expression).unwrap(), Value::Integer(expected));
    }
    assert!(
        s.services
            .messages
            .last()
            .unwrap()
            .contains("opener unavailable")
    );
    assert_eq!(s.evaluate("1+2").unwrap(), Value::Integer(3));
}
