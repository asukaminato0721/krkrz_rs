use krkrz_runtime::Session;
use krkrz_tjs::{Value, VmAbort};
use serde::Deserialize;
#[derive(Deserialize)]
struct Case {
    name: String,
    source: String,
    expected: Value,
}
fn execute(source: &str) -> anyhow::Result<Value> {
    let project = tempfile::tempdir()?;
    let saves = tempfile::tempdir()?;
    std::fs::write(project.path().join("case.tjs"), source)?;
    Session::open(project.path(), Some(saves.path()), None, 100_000)?.execute_storage("case.tjs")
}
#[test]
fn original_menu_model_corpus() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/menu.json")).unwrap();
    for c in cases {
        let mut modes = vec![("plugin", c.source.as_str())];
        // Relink cases require a preceding explicit link; the remaining model
        // corpus also applies to legacy scripts using built-in menus directly.
        if let Some(core) = c.source.strip_prefix("Plugins.link('menu.dll');")
            && !core.contains("Plugins.link(")
        {
            modes.push(("built-in", core));
        }
        for (mode, source) in modes {
            assert_eq!(
                execute(source).unwrap_or_else(|e| panic!("{} ({mode}): {e:#}", c.name)),
                c.expected,
                "{} ({mode})",
                c.name
            );
        }
    }
}

#[test]
fn builtin_menus_support_kag_subclasses_and_explicit_linking() {
    assert_eq!(
        execute(
            r#"
        class GameMenu extends MenuItem {
            var _enabled = true;
            function GameMenu(w, caption) { super.MenuItem(w, caption); }
            property enabled {
                getter { return _enabled; }
                setter(v) { _enabled = v; super.enabled = v; }
            }
        }
        class GameWindow extends Window {
            var item;
            function GameWindow() {
                super.Window();
                menu.add(item = new GameMenu(this, 'System'));
            }
        }
        var w = new GameWindow(), root = w.menu, original = MenuItem;
        w.item.enabled = false;
        var table = MenuItem.textToKeycode;
        var initiallyEmpty = Plugins.getList().count == 0;
        Plugins.link('menu.dll');
        return [initiallyEmpty, MenuItem === original, table === MenuItem.textToKeycode,
            w.menu === root, root.children[0] === w.item, w.item.parent === root,
            w.item.enabled, Plugins.getList().join(',')].join('|');
    "#
        )
        .unwrap(),
        Value::string("1|1|1|1|1|1|0|menu.dll")
    );
}
#[test]
fn pending_clicks_are_batched_and_cancelled_on_invalidation() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("start.tjs"), "Plugins.link('menu.dll');var w=new Window(),m=new MenuItem(w,'a'),n=new MenuItem(w,'b'),count=0;w.menu.add(m);w.menu.add(n);m.onClick=function(){count++;m.fireClick();invalidate n;};n.onClick=function(){count+=100;};m.fireClick();n.fireClick();").unwrap();
    std::fs::write(project.path().join("count.tjs"), "return count;").unwrap();
    std::fs::write(project.path().join("stop.tjs"), "invalidate m;").unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), None, 100_000).unwrap();
    session.execute_storage("start.tjs").unwrap();
    assert_eq!(
        session.execute_storage("count.tjs").unwrap(),
        Value::Integer(0)
    );
    session.dispatch_events().unwrap();
    assert_eq!(
        session.execute_storage("count.tjs").unwrap(),
        Value::Integer(1)
    );
    session.dispatch_events().unwrap();
    assert_eq!(
        session.execute_storage("count.tjs").unwrap(),
        Value::Integer(2)
    );
    session.execute_storage("stop.tjs").unwrap();
    session.dispatch_events().unwrap();
    assert_eq!(
        session.execute_storage("count.tjs").unwrap(),
        Value::Integer(2)
    );
}
#[test]
fn invalid_hierarchies_fail_and_presentation_is_explicit() {
    assert_eq!(execute("Plugins.link('menu.dll');var w=new Window(),a=new MenuItem(w),b=new MenuItem(w),n=0;a.add(b);try{b.add(a);}catch(e){n++;}try{a.insert(new MenuItem(w),99);}catch(e){n++;}return [n,a.children.count,b.parent===a].join('|');").unwrap(), Value::string("2|1|1"));
    for source in ["m.popup(0,0,0)", "m.HMENU"] {
        let err = execute(&format!("Plugins.link('menu.dll');var w=new Window(),m=w.menu;try{{{source};}}catch(e){{return 999;}}")).unwrap_err();
        assert!(err.downcast_ref::<VmAbort>().is_some(), "{err:#}");
    }
}
#[test]
fn clicks_require_enabled_ancestors_and_a_window_root() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("start.tjs"), "Plugins.link('menu.dll');var w=new Window(),p=new MenuItem(w),m=new MenuItem(w),count=0;m.onClick=function(){count++;};p.add(m);m.fireClick();w.menu.add(p);p.enabled=0;m.fireClick();p.enabled=1;m.visible=0;m.fireClick();").unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), None, 100_000).unwrap();
    session.execute_storage("start.tjs").unwrap();
    session.dispatch_events().unwrap();
    assert_eq!(session.evaluate("count").unwrap(), Value::Integer(1));
}
#[test]
fn callbacks_share_the_execution_budget() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("start.tjs"), "Plugins.link('menu.dll');var w=new Window(),m=new MenuItem(w);w.menu.add(m);m.onClick=function(){while(true){}};m.fireClick();").unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), None, 1000).unwrap();
    session.execute_storage("start.tjs").unwrap();
    let err = session.dispatch_events().unwrap_err();
    assert!(err.downcast_ref::<VmAbort>().is_some(), "{err:#}");
    assert!(format!("{err:#}").contains("budget"), "{err:#}");
}
