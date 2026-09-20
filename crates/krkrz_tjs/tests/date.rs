use krkrz_tjs::{Value, Vm, compile};
use serde::Deserialize;

#[test]
fn original_date_corpus() {
    // Match the isolated Windows oracle's timezone without changing the process
    // environment while other Rust tests run.
    if std::env::var_os("KRKRZ_DATE_TEST_CHILD").is_none() {
        assert!(
            std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "original_date_corpus", "--nocapture"])
                .env("TZ", "Asia/Tokyo")
                .env("KRKRZ_DATE_TEST_CHILD", "1")
                .status()
                .unwrap()
                .success()
        );
        return;
    }
    #[derive(Deserialize)]
    struct Case {
        name: String,
        source: String,
        expected: Value,
    }
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/date.json")).unwrap();
    for case in cases {
        let program = compile(&case.name, &case.source).unwrap();
        let value = Vm::default()
            .execute(&program, &mut (), &mut 100_000)
            .unwrap_or_else(|e| panic!("{}: {e:#}", case.name));
        assert_eq!(value, case.expected, "{}", case.name);
    }
}
