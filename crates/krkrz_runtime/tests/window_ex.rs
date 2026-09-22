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
    Session::open(project.path(), Some(saves.path()), None, budget)?.execute_storage("case.tjs")
}
#[test]
fn original_window_ex_corpus() {
    let mut cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/window_ex.json")).unwrap();
    cases.extend(
        serde_json::from_str::<Vec<Case>>(include_str!("fixtures/menu_appearance.json")).unwrap(),
    );
    cases.extend(
        serde_json::from_str::<Vec<Case>>(include_str!("fixtures/window_rects.json")).unwrap(),
    );
    for c in cases {
        assert_eq!(
            run(&c.source, 500_000).unwrap_or_else(|e| panic!("{}: {e:#}", c.name)),
            c.expected,
            "{}",
            c.name
        );
    }
}
const SETUP: &str = "Plugins.link('menu.dll');if(typeof global.Pad=='undefined')global.Pad=%[];if(typeof Debug.console=='undefined')Debug.console=%[];Plugins.link('windowEx.dll');";

#[test]
fn deferred_maximize_minimize_restore_and_query_veto() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("startup.tjs"),format!("{SETUP}var w=new Window();w.setPos(40,50);w.setSize(320,240);w.registerExEvent();var events=[],veto=true;w.onMaximizeQuery=function(){{global.events.add('query');return global.veto;}};w.onMaximize=function(){{global.events.add('max');}};w.onMinimize=function(){{global.events.add('min');}};w.onShow=function(){{global.events.add('show');}};")).unwrap();
    let mut s = Session::open(project.path(), Some(saves.path()), None, 100_000).unwrap();
    s.startup().unwrap();
    s.evaluate("w.maximize()").unwrap();
    assert_eq!(s.evaluate("w.maximized").unwrap(), Value::Integer(0));
    s.tick(1).unwrap();
    assert_eq!(s.evaluate("w.maximized").unwrap(), Value::Integer(0));
    s.evaluate("veto=false").unwrap();
    s.evaluate("w.maximized=true").unwrap();
    s.tick(2).unwrap();
    assert_eq!(s.evaluate("w.maximized").unwrap(), Value::Integer(1));
    assert_eq!(
        s.evaluate("w.getNormalRect().w").unwrap(),
        Value::Integer(320)
    );
    assert_eq!(
        s.evaluate("w.width==System.desktopWidth").unwrap(),
        Value::Integer(1)
    );
    s.evaluate("w.minimize()").unwrap();
    s.tick(3).unwrap();
    assert_eq!(s.evaluate("w.minimized").unwrap(), Value::Integer(1));
    s.evaluate("w.showRestore()").unwrap();
    s.tick(4).unwrap();
    assert_eq!(s.evaluate("w.maximized").unwrap(), Value::Integer(1));
    s.evaluate("w.maximized=false").unwrap();
    s.tick(5).unwrap();
    assert_eq!(
        s.evaluate("[w.left,w.top,w.width,w.height].join(',')")
            .unwrap(),
        Value::string("40,50,320,240")
    );
    assert_eq!(
        s.evaluate("events.join(',')").unwrap(),
        Value::string("query,query,max,min,show,max")
    );
}

#[test]
fn linux_host_reports_dwm_preview_unavailable() {
    assert_eq!(
        run(
            &format!("{SETUP}return System.setIconicPreview(true);"),
            10_000
        )
        .unwrap(),
        Value::Integer(0)
    );
}

#[test]
fn menu_bitmap_is_an_independent_alpha_thresholded_snapshot() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("case.tjs"), format!("{SETUP}global.w=new Window();global.m=new MenuItem(w,'item');w.menu.add(m);global.p=new Layer(w,null);p.setSize(2,1);p.setImageSize(2,1);p.setMainPixel(0,0,0x123456);p.setMainPixel(1,0,0xabcdef);p.setMaskPixel(0,0,63);p.setMaskPixel(1,0,64);m.bmpItem=p;m.rightJustify=true;p.setMainPixel(0,0,0);invalidate p;")).unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), None, 100_000).unwrap();
    session.execute_storage("case.tjs").unwrap();
    let menu = session.evaluate("m").unwrap();
    let appearance = session.menu_appearance(&menu).unwrap();
    assert!(appearance.right_justify);
    let krkrz_runtime::MenuBitmap::Image(image) = &appearance.bitmaps[0] else {
        panic!("expected bitmap");
    };
    assert_eq!(image.rgba, [0x12, 0x34, 0x56, 0, 0xab, 0xcd, 0xef, 255]);
    session.evaluate("m.bmpItem=8").unwrap();
    assert!(matches!(
        session.menu_appearance(&menu).unwrap().bitmaps[0],
        krkrz_runtime::MenuBitmap::System(8)
    ));
    session.evaluate("invalidate m").unwrap();
    assert!(session.menu_appearance(&menu).is_err());
}

#[test]
fn menu_extension_parent_access_preserves_budget_and_lifetime_checks() {
    for getter in ["invalidate global.m;return null;", "while(true){}"] {
        let source = format!(
            "{SETUP}class M extends MenuItem{{function M(w){{super.MenuItem(w,'item');}}property parent{{getter(){{{getter}}}}}}}global.m=new M(new Window());return m.bmpItem;"
        );
        let error = run(&source, 2_000).unwrap_err();
        if getter.starts_with("while") {
            assert!(error.downcast_ref::<VmAbort>().is_some(), "{error:#}");
        }
    }
}

#[test]
fn system_menu_snapshot_reset_and_selection_callback() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("case.tjs"), format!(
        "{SETUP}global.w=new Window();w.registerExEvent();global.selected=void;w.onExSystemMenuSelected=function(item){{global.selected=item.state;return 7;}};global.leaf=%[caption:'first',state:42,checked:1,group:1];global.disabled=%[caption:'disabled',state:99,enabled:0];w.exSystemMenu=[%[caption:'hidden',visible:0],%[caption:'-'],%[caption:'parent',children:[leaf,disabled]]];"
    )).unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), None, 50_000).unwrap();
    session.execute_storage("case.tjs").unwrap();
    let window = session.evaluate("w").unwrap();
    let items = session.system_menu(&window).unwrap();
    assert_eq!(items.len(), 2);
    assert!(items[0].command.is_none());
    let leaf = &items[1].children[0];
    assert_eq!(leaf.caption, "first");
    assert!(leaf.checked && leaf.radio);
    let command = leaf.command.unwrap();
    assert_eq!(command, 0xefff);
    assert!(session.select_system_menu(&window, command - 1).is_err());
    assert_eq!(
        session.select_system_menu(&window, command).unwrap(),
        Value::Integer(7)
    );
    assert_eq!(session.evaluate("selected").unwrap(), Value::Integer(42));
    session
        .evaluate("(leaf.caption='updated',leaf.state=43)")
        .unwrap();
    assert_eq!(
        session.system_menu(&window).unwrap()[1].children[0].caption,
        "first"
    );
    session.select_system_menu(&window, command).unwrap();
    assert_eq!(session.evaluate("selected").unwrap(), Value::Integer(43));
    session.evaluate("w.resetExSystemMenu()").unwrap();
    assert_eq!(
        session.system_menu(&window).unwrap()[1].children[0].caption,
        "updated"
    );
    session.evaluate("w.exSystemMenu=null").unwrap();
    assert!(session.system_menu(&window).unwrap().is_empty());
    assert!(session.select_system_menu(&window, command).is_err());
    session.evaluate("invalidate w").unwrap();
    assert!(session.system_menu(&window).is_err());
}

#[test]
fn system_menu_callbacks_share_limits_and_handle_invalidation() {
    for body in [
        "var a=%[caption:'cycle'];a.children=[a];w.exSystemMenu=[a];",
        "class C{property caption{getter(){while(1){}}}}w.exSystemMenu=[new C()];",
    ] {
        let error = run(
            &format!("{SETUP}var w=new Window();w.registerExEvent();{body}"),
            2000,
        )
        .unwrap_err();
        assert!(error.downcast_ref::<VmAbort>().is_some(), "{error:#}");
    }
    assert_eq!(run(&format!("{SETUP}global.w=new Window();w.registerExEvent();class C{{property caption{{getter(){{invalidate global.w;return 'gone';}}}}}}try{{w.exSystemMenu=[new C()];}}catch(e){{return isvalid w;}}return 99;"), 10_000).unwrap(), Value::Integer(0));
}
#[test]
fn callbacks_share_budget_and_handle_reentrant_invalidation() {
    let source = format!(
        "{SETUP}class W extends Window{{function W(){{super.Window();}}property onMove{{getter(){{invalidate this;return void;}}}}}}var w=new W();try{{w.registerExEvent();}}catch(e){{return isvalid w;}}return 99;"
    );
    assert_eq!(run(&source, 100_000).unwrap(), Value::Integer(0));
    let source = format!(
        "{SETUP}class W extends Window{{function W(){{super.Window();}}property onMove{{getter(){{while(true){{}}}}}}}}var w=new W();try{{w.registerExEvent();}}catch(e){{return 99;}}"
    );
    let error = run(&source, 2000).unwrap_err();
    assert!(error.downcast_ref::<VmAbort>().is_some(), "{error:#}");
}
#[test]
fn native_window_operations_are_explicit_until_host_support_exists() {
    for operation in ["w.focusMenuByKey(0)", "w.setOverlayBitmap(null)"] {
        let error = run(
            &format!("{SETUP}var w=new Window();try{{{operation};}}catch(e){{return 99;}}"),
            100_000,
        )
        .unwrap_err();
        assert!(
            error.downcast_ref::<VmAbort>().is_some(),
            "{operation}: {error:#}"
        );
    }
}
