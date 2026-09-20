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

#[test]
fn local_dst_normalization() {
    if std::env::var_os("KRKRZ_DST_TEST_CHILD").is_none() {
        for zone in ["America/New_York", "Australia/Lord_Howe"] {
            assert!(
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "local_dst_normalization", "--nocapture"])
                    .env("TZ", zone)
                    .env("KRKRZ_DST_TEST_CHILD", "1")
                    .status()
                    .unwrap()
                    .success(),
                "DST normalization in {zone}"
            );
        }
        return;
    }
    // TJS supplies tm_isdst=0 at construction, then preserves the previous
    // daylight adjustment in setters. This intentionally differs from choosing
    // the new date's wall-clock offset automatically.
    let (source, expected) = match std::env::var("TZ").unwrap().as_str() {
        "America/New_York" => (
            "var d=new Date(2024,0,15,12); var a=[d.getHours()];
             d.setMonth(6); a.add(d.getHours()); d.setMonth(0); a.add(d.getHours());
             var gap=new Date(2024,2,10,2,30); a.add(gap.getHours());
             var fold=new Date(2024,10,3,1,30); a.add(fold.getTime());
             return a.join('|');",
            "12|13|12|3|1730615400000",
        ),
        "Australia/Lord_Howe" => (
            "var d=new Date(2024,0,15,12); var a=[d.getHours(),d.getMinutes()];
             d.setMonth(6); a.add(d.getHours()); a.add(d.getMinutes());
             return a.join('|');",
            "12|30|12|0",
        ),
        zone => panic!("unexpected test timezone {zone}"),
    };
    let value = Vm::default()
        .execute(&compile("dst", source).unwrap(), &mut (), &mut 100_000)
        .unwrap();
    assert_eq!(value.text(), expected);
}
