use krkrz_runtime::Session;
use krkrz_tjs::{Value, VmAbort};
use serde::Deserialize;
#[derive(Deserialize)]
struct Case {
    name: String,
    source: String,
    expected: Value,
}
fn run(source: &str, budget: u64) -> anyhow::Result<Value> {
    let project = tempfile::tempdir()?;
    let saves = tempfile::tempdir()?;
    std::fs::write(project.path().join("case.tjs"), source)?;
    Session::open(project.path(), Some(saves.path()), false, budget)?.execute_storage("case.tjs")
}
#[test]
fn original_dialog_corpus() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/dialog.json")).unwrap();
    for c in cases {
        assert_eq!(
            run(&c.source, 500_000).unwrap_or_else(|e| panic!("{}: {e:#}", c.name)),
            c.expected,
            "{}",
            c.name
        );
    }
}
#[test]
fn buffers_have_checked_bounds_and_no_process_pointers() {
    assert_eq!(run("Plugins.link('win32dialog.dll');var b=new WIN32Dialog.Blob(8),n=0;try{b.setDWord(6,1);}catch(e){n++;}try{b.getByte(-1);}catch(e){n++;}try{new WIN32Dialog.Blob(-1);}catch(e){n++;}return [n,b.getDWord(4)].join('|');",100_000).unwrap(),Value::string("3|0"));
    for op in [
        "WIN32Dialog.Blob.ReferPointer(1)",
        "b.pointer",
        "b.setText(0,'x')",
        "WIN32Dialog.messageBox(null,'x','y',0)",
        "d.open()",
    ] {
        let err=run(&format!("Plugins.link('win32dialog.dll');var b=new WIN32Dialog.Blob(8),d=new WIN32Dialog(null);try{{{op};}}catch(e){{return 99;}}"),100_000).unwrap_err();
        assert!(err.downcast_ref::<VmAbort>().is_some(), "{op}: {err:#}");
    }
}
#[test]
fn template_getters_share_budget_and_cannot_invalidate_receiver_unsafely() {
    let source = "Plugins.link('win32dialog.dll');var h=new WIN32Dialog.Header();class C{property x{getter(){invalidate h;return 2;}}}try{h.store(new C());}catch(e){return isvalid h;}return 99;";
    assert_eq!(run(source, 100_000).unwrap(), Value::Integer(0));
    let err=run("Plugins.link('win32dialog.dll');var h=new WIN32Dialog.Header();class C{property title{getter(){while(true){}}}}try{h.store(new C());}catch(e){return 99;}",2000).unwrap_err();
    assert!(err.downcast_ref::<VmAbort>().is_some(), "{err:#}");
}

#[test]
fn caught_native_errors_keep_messages_separate_from_trace() {
    assert_eq!(run("Plugins.link('win32dialog.dll');var d=new WIN32Dialog(null);try{d.progressValue;}catch(e){return e.message=='dialog is not progress mode.' && e.trace.indexOf('case.tjs')>=0;}",100_000).unwrap(),Value::Integer(1));
}
