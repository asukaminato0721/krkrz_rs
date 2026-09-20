use krkrz_runtime::{InputEvent, Session};
use krkrz_tjs::Value;
fn session(source: &str) -> (tempfile::TempDir, tempfile::TempDir, Session, Value) {
    let project = tempfile::tempdir().unwrap();
    let save = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("startup.tjs"), source).unwrap();
    let mut session = Session::open(project.path(), Some(save.path()), false, 10_000_000).unwrap();
    session.startup().unwrap();
    let window = session.evaluate("w").unwrap();
    (project, save, session, window)
}
const SETUP: &str = r#"
var log=[];
class TestWindow extends Window {
 function TestWindow(){super.Window();visible=true;setInnerSize(200,100);}
 function action(e) { if(e.target===this) {if(e.type=="onMouseDown") global.log.add("window:"+e.x+":"+e.y);} else if(e.type!="onPaint" && e.type!="onHitTest") global.log.add(e.type+":"+e.x+":"+e.y); }
}
var w=new TestWindow(),p=new Layer(w,null);p.setSize(100,50);p.visible=true;w.setZoom(2,1);
var owner=%[action:function(e){if(e.type!="onPaint" && e.type!="onHitTest") global.log.add(e.type+":"+e.x+":"+e.y);}];
var a=new Layer(w,p,owner);a.setSize(20,20);a.setPos(10,5);a.visible=true;a.hitThreshold=0;a.focusable=true;
"#;
#[test]
fn pointer_capture_click_coordinates_and_disabled_occlusion() {
    let (_p, _s, mut s, w) = session(SETUP);
    s.input(
        &w,
        InputEvent::PointerDown {
            x: 24,
            y: 14,
            button: 0,
            shift: 8,
        },
    )
    .unwrap();
    assert_eq!(
        s.evaluate("log.join('|')").unwrap(),
        Value::string("window:24:14|onMouseEnter::|onMouseMove:2:2|onMouseDown:2:2")
    );
    assert_eq!(
        s.evaluate("System.getKeyState(1)").unwrap(),
        Value::Integer(1)
    );
    s.input(
        &w,
        InputEvent::PointerMove {
            x: 100,
            y: 80,
            shift: 8,
        },
    )
    .unwrap();
    assert_eq!(
        s.evaluate("log[log.count-1]").unwrap(),
        Value::string("onMouseMove:40:35")
    );
    s.input(
        &w,
        InputEvent::PointerUp {
            x: 100,
            y: 80,
            button: 0,
            shift: 0,
        },
    )
    .unwrap();
    assert_eq!(
        s.evaluate("System.getKeyState(1)").unwrap(),
        Value::Integer(0)
    );
    s.evaluate("log.clear()").unwrap();
    s.input(
        &w,
        InputEvent::PointerDown {
            x: 24,
            y: 14,
            button: 0,
            shift: 8,
        },
    )
    .unwrap();
    s.input(&w, InputEvent::Click { x: 24, y: 14 }).unwrap();
    s.input(
        &w,
        InputEvent::PointerUp {
            x: 24,
            y: 14,
            button: 0,
            shift: 0,
        },
    )
    .unwrap();
    assert_eq!(
        s.evaluate("log.find('onClick:2:2')>=0").unwrap(),
        Value::Integer(1)
    );
    s.evaluate("(a.enabled=false,log.clear())").unwrap();
    s.input(
        &w,
        InputEvent::PointerDown {
            x: 24,
            y: 14,
            button: 0,
            shift: 8,
        },
    )
    .unwrap();
    assert_eq!(
        s.evaluate("log.find('onMouseDown:2:2')").unwrap(),
        Value::Integer(-1)
    );
}
#[test]
fn explicit_hit_rejection_focus_key_and_close_query() {
    let (_p, _s, mut s, w) = session(SETUP);
    s.evaluate("(global.oldHit=a.onHitTest, a.onHitTest=function(x,y,hit){(global.Layer.onHitTest incontextof this)(x,y,false);})").unwrap();
    s.input(
        &w,
        InputEvent::PointerDown {
            x: 24,
            y: 14,
            button: 0,
            shift: 8,
        },
    )
    .unwrap();
    assert_eq!(
        s.evaluate("log.find('onMouseDown:2:2')").unwrap(),
        Value::Integer(-1)
    );
    s.evaluate("(a.onHitTest=oldHit,a.focus(),log.clear())")
        .unwrap();
    s.input(&w, InputEvent::KeyDown { key: 65, shift: 0 })
        .unwrap();
    assert_eq!(
        s.evaluate("System.getKeyState(65)").unwrap(),
        Value::Integer(1)
    );
    assert_eq!(s.evaluate("log[0]").unwrap(), Value::string("onKeyDown::"));
    s.input(&w, InputEvent::Focus(false)).unwrap();
    assert_eq!(
        s.evaluate("System.getKeyState(65)").unwrap(),
        Value::Integer(0)
    );
    s.evaluate("(global.oldClose=w.onCloseQuery, w.onCloseQuery=function(canClose){(global.Window.onCloseQuery incontextof this)(false);})").unwrap();
    s.request_window_close(&w).unwrap();
    assert_eq!(s.evaluate("isvalid w").unwrap(), Value::Integer(1));
    s.evaluate("w.onCloseQuery=oldClose").unwrap();
    s.request_window_close(&w).unwrap();
    assert!(s.services.windows.is_empty());
}
#[test]
fn resize_event_and_recent_key_state() {
    let (_p, _s, mut s, w) = session(SETUP);
    s.evaluate("w.onResize=function(){global.log.add('resize:'+innerWidth+':'+innerHeight);}")
        .unwrap();
    s.resize_window(&w, 300, 150).unwrap();
    s.dispatch_events().unwrap();
    assert_eq!(
        s.evaluate("log.join('|')").unwrap(),
        Value::string("resize:300:150")
    );
    s.input(&w, InputEvent::KeyDown { key: 32, shift: 0 })
        .unwrap();
    s.input(&w, InputEvent::KeyUp { key: 32, shift: 0 })
        .unwrap();
    assert_eq!(
        s.evaluate("System.getKeyState(32,false)").unwrap(),
        Value::Integer(1)
    );
    assert_eq!(
        s.evaluate("System.getKeyState(32,false)").unwrap(),
        Value::Integer(0)
    );
}

#[test]
fn temporary_cursor_hiding_recovers_on_motion() {
    let (_p, _s, mut session, window) = session(SETUP);
    session
        .input(
            &window,
            InputEvent::PointerMove {
                x: 24,
                y: 14,
                shift: 0,
            },
        )
        .unwrap();
    session
        .evaluate("(a.cursor=crHandPoint,w.hideMouseCursor())")
        .unwrap();
    assert_eq!(session.window_cursor(&window).unwrap(), -21);
    assert_eq!(
        session.evaluate("w.mouseCursorState").unwrap(),
        Value::Integer(1)
    );
    session
        .input(
            &window,
            InputEvent::PointerMove {
                x: 28,
                y: 14,
                shift: 0,
            },
        )
        .unwrap();
    assert_eq!(
        session.evaluate("w.mouseCursorState").unwrap(),
        Value::Integer(0)
    );
    session.evaluate("w.mouseCursorState=mcsHidden").unwrap();
    session
        .input(
            &window,
            InputEvent::PointerMove {
                x: 30,
                y: 14,
                shift: 0,
            },
        )
        .unwrap();
    assert_eq!(
        session.evaluate("w.mouseCursorState").unwrap(),
        Value::Integer(2)
    );
}

#[test]
fn callbacks_can_destroy_the_window_during_hit_testing() {
    let (_p, _s, mut session, window) = session(SETUP);
    session
        .evaluate("a.onMouseEnter=function(){invalidate global.w;}")
        .unwrap();
    session
        .input(
            &window,
            InputEvent::PointerDown {
                x: 24,
                y: 14,
                button: 0,
                shift: 8,
            },
        )
        .unwrap();
    assert!(session.services.windows.is_empty());
}
