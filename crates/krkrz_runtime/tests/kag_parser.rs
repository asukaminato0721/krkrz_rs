use krkrz_runtime::Session;
use krkrz_tjs::Value;
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    name: String,
    source: String,
    expected: Value,
}
fn session(source: &str, budget: u64) -> (tempfile::TempDir, tempfile::TempDir, Session) {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("case.tjs"), source).unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), false, budget).unwrap();
    session.execute_storage("case.tjs").unwrap();
    (project, saves, session)
}
#[test]
fn original_kag_parser_corpus() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/kag_parser.json")).unwrap();
    for case in cases {
        let (project, saves, mut session) = session("", 100_000);
        std::fs::write(project.path().join("test.tjs"), &case.source).unwrap();
        assert_eq!(
            session
                .execute_storage("test.tjs")
                .unwrap_or_else(|e| panic!("{}: {e:#}", case.name)),
            case.expected,
            "{}",
            case.name
        );
        drop(saves);
    }
}

#[test]
fn saved_call_resumes_in_fresh_session_and_survives_inserted_lines() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("story.ks"),
        "*a\n[call target=*sub]R[p]\n[end]\n*sub\nS[return]",
    )
    .unwrap();
    let setup = "Plugins.link('KAGParserEx.dll');global.p=new KAGParser();";
    std::fs::write(project.path().join("setup.tjs"), setup).unwrap();
    {
        let mut session =
            Session::open(project.path(), Some(saves.path()), false, 100_000).unwrap();
        session.execute_storage("setup.tjs").unwrap();
        assert_eq!(
            session
                .evaluate(
                    "Scripts.exec('p.loadScenario(\"story.ks\");return p.getNextTag().text;')"
                )
                .unwrap(),
            Value::string("S")
        );
        session.evaluate("Scripts.exec('var s=p.store();(Dictionary.saveStruct incontextof s)(System.dataPath+\"parser.tjs\");')").unwrap();
    }
    assert!(saves.path().join("parser.tjs").exists());
    assert!(!project.path().join("parser.tjs").exists());
    // Calls use a label plus an offset, so edits before the label are safe.
    std::fs::write(
        project.path().join("story.ks"),
        ";new comment\n\n*a\n[call target=*sub]R[p]\n[end]\n*sub\nS[return]",
    )
    .unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), false, 100_000).unwrap();
    session.execute_storage("setup.tjs").unwrap();
    assert_eq!(session.evaluate("Scripts.exec('p.restore(Scripts.evalStorage(System.dataPath+\"parser.tjs\"));var a=p.getNextTag().text;var b=p.getNextTag().text;return a+b+\":\"+p.callStackDepth;')").unwrap(),Value::string("SR:0"));
}

#[test]
fn recursion_and_infinite_jumps_share_the_vm_budget() {
    for scenario in [
        "*loop\n[jump target=*loop]",
        "[macro name=m][m][endmacro][m]",
    ] {
        let (project, _saves, mut session) = session("", 10_000);
        std::fs::write(project.path().join("loop.ks"), scenario).unwrap();
        std::fs::write(project.path().join("test.tjs"), "Plugins.link('KAGParserEx.dll');var p=new KAGParser();p.loadScenario('loop.ks');try {p.getNextTag();}catch{return 'caught';}").unwrap();
        let error = session.execute_storage("test.tjs").unwrap_err();
        assert!(
            error.downcast_ref::<krkrz_tjs::VmAbort>().is_some(),
            "{error:#}"
        );
    }
}

#[test]
fn callbacks_can_clear_or_invalidate_parser_without_panicking() {
    for source in [
        "p.onScenarioLoad=function(){return '*a\\nX';};p.onLabel=function(){invalidate p;};",
        "p.onScenarioLoad=function(){return '[emb exp=\"this.clear()\"]';};",
    ] {
        let (project, _saves, mut session) = session("", 10_000);
        std::fs::write(project.path().join("test.tjs"), format!("Plugins.link('KAGParserEx.dll');global.p=new KAGParser();{source}p.loadScenario('x');return p.getNextTag();")).unwrap();
        assert!(session.execute_storage("test.tjs").is_err());
    }
}

#[test]
fn restore_rejects_corrupted_stacks_and_changed_return_line() {
    for mutation in [
        "s.macroArgStackDepth=1000000000;",
        "s.IfLevel=2;s.ExcludeLevelStack='ffffffff';s.IfLevelExecutedStack='11';",
        "s.callStack[0].offset=1000000000;",
        "s.callStack[0].orgLineStr='different';",
    ] {
        let (project, _saves, mut session) = session("", 100_000);
        std::fs::write(
            project.path().join("test.tjs"),
            format!(
                r#"
Plugins.link('KAGParserEx.dll');var p=new KAGParser();
p.onScenarioLoad=function(){{return '*a\n[call target=*sub]R\n*sub\nS[return]';}};
p.loadScenario('x');p.getNextTag();var s=p.store();{mutation}
p.restore(s);p.getNextTag();p.getNextTag();
"#
            ),
        )
        .unwrap();
        assert!(session.execute_storage("test.tjs").is_err(), "{mutation}");
    }
}
