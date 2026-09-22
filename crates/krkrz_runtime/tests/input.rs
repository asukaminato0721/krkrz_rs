use krkrz_runtime::{InputEvent, Session};
use krkrz_tjs::Value;
fn session(source: &str) -> (tempfile::TempDir, tempfile::TempDir, Session, Value) {
    let project = tempfile::tempdir().unwrap();
    let save = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("startup.tjs"), source).unwrap();
    let mut session = Session::open(project.path(), Some(save.path()), None, 10_000_000).unwrap();
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
fn host_resizable_canvas_fits_content_and_transforms_pointer_coordinates() {
    let (_project, _saves, mut s, w) = session(
        r#"
        var w=new Window(), p=new Layer(w,null), a=new Layer(w,p), log=[];
        w.visible=true;w.borderStyle=bsSingle;w.setInnerSize(100,50);
        p.setSize(100,50);a.setSize(30,20);a.setPos(30,10);
        a.visible=true;a.hitThreshold=0;
        a.onMouseDown=function(x,y,button,shift){log.add(x+','+y);};
    "#,
    );
    let Value::Object(reference) = &w else {
        panic!("expected Window")
    };
    let id = reference.object.unwrap();
    s.services.windows.get_mut(&id).unwrap().fit_to_client = true;
    for (size, rect) in [
        ([300, 100], [50, 0, 200, 100]),
        ([100, 100], [0, 25, 100, 50]),
        ([50, 25], [0, 0, 50, 25]),
    ] {
        s.resize_window(&w, size[0], size[1]).unwrap();
        assert_eq!(s.services.windows[&id].draw_rect(100, 50), rect);
        let x = rect[0] + rect[2] * 40 / 100;
        let y = rect[1] + rect[3] * 20 / 50;
        s.input(
            &w,
            InputEvent::PointerDown {
                x,
                y,
                button: 0,
                shift: 0,
            },
        )
        .unwrap();
        s.input(
            &w,
            InputEvent::PointerUp {
                x,
                y,
                button: 0,
                shift: 0,
            },
        )
        .unwrap();
    }
    assert_eq!(
        s.evaluate("log.join('|')").unwrap(),
        Value::string("10,10|10,10|10,10")
    );
    assert_eq!(
        s.evaluate("p.width+','+p.height").unwrap(),
        Value::string("100,50")
    );
    // Full screen must fit the canvas, even after a resize changed the client aspect.
    s.resize_window(&w, 100, 100).unwrap();
    s.evaluate("w.fullScreen=true").unwrap();
    assert_eq!(
        s.services.windows[&id].draw_rect(100, 50),
        [0, 40, 1280, 640]
    );
    s.evaluate("w.fullScreen=false").unwrap();
    assert_eq!(s.services.windows[&id].draw_rect(100, 50), [0, 25, 100, 50]);
}

#[test]
fn modal_layers_isolate_clicks_and_preserve_original_capture_and_focus_dispatch() {
    let (_project, _saves, mut s, w) = session(
        r#"
        var log=[], w=new Window(), p=new Layer(w,null), a=new Layer(w,p), b=new Layer(w,p);
        w.visible=true;w.setInnerSize(100,100);p.setSize(100,100);
        a.setSize(50,100);b.setSize(50,100);b.setPos(50,0);
        a.visible=b.visible=a.focusable=b.focusable=true;a.hitThreshold=b.hitThreshold=0;
        a.onClick=function(){log.add('a-click');};b.onClick=function(){log.add('b-click');};
        a.onKeyDown=function(){log.add('a-key');};b.onKeyDown=function(){log.add('b-key');};
        b.onMouseMove=function(){log.add('b-move');};
    "#,
    );
    let down = |x| InputEvent::PointerDown {
        x,
        y: 20,
        button: 0,
        shift: 8,
    };
    s.input(&w, down(60)).unwrap(); // Establish capture before opening the dialog.
    s.evaluate("(a.setMode(),log.clear())").unwrap();
    s.input(
        &w,
        InputEvent::PointerMove {
            x: 80,
            y: 20,
            shift: 8,
        },
    )
    .unwrap();
    s.input(&w, InputEvent::Click { x: 60, y: 20 }).unwrap();
    s.input(&w, InputEvent::KeyDown { key: 65, shift: 0 })
        .unwrap();
    s.input(
        &w,
        InputEvent::PointerUp {
            x: 80,
            y: 20,
            button: 0,
            shift: 0,
        },
    )
    .unwrap();
    s.input(&w, down(20)).unwrap();
    s.input(&w, InputEvent::Click { x: 20, y: 20 }).unwrap();
    assert_eq!(
        s.evaluate("log.join('|')").unwrap(),
        Value::string("b-move|a-key|a-click")
    );

    s.evaluate("(b.setMode(),b.focus(),log.clear())").unwrap();
    s.input(&w, down(20)).unwrap();
    s.input(&w, InputEvent::Click { x: 20, y: 20 }).unwrap();
    s.input(&w, InputEvent::KeyDown { key: 65, shift: 0 })
        .unwrap();
    assert_eq!(s.evaluate("log.join('|')").unwrap(), Value::string("b-key"));
    s.evaluate("(b.removeMode(),log.clear())").unwrap();
    // Original dispatch retains focus after pop even when that layer becomes
    // node-disabled. Do not redirect or drop its key events.
    assert_eq!(
        s.evaluate("b.focused && !b.nodeEnabled").unwrap(),
        Value::Integer(1)
    );
    s.input(&w, InputEvent::KeyDown { key: 65, shift: 0 })
        .unwrap();
    assert_eq!(s.evaluate("log.join('|')").unwrap(), Value::string("b-key"));
    s.evaluate("invalidate a").unwrap();
    s.input(&w, down(60)).unwrap();
    s.input(&w, InputEvent::Click { x: 60, y: 20 }).unwrap();
    assert_eq!(
        s.evaluate("log[log.count-1]").unwrap(),
        Value::string("b-click")
    );
}
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

#[test]
fn get_layer_at_searches_the_whole_tree_in_caller_coordinates() {
    let (_p, _s, mut s, _) = session(
        r#"
        var w=new Window(), p=new Layer(w,null), a=new Layer(w,p), b=new Layer(w,p);
        p.setSize(100,100); a.setSize(20,20); b.setSize(20,20);
        a.setPos(10,15); b.setPos(10,15);
        a.visible=b.visible=true; a.hitThreshold=b.hitThreshold=0;
        var c=new Layer(w,b); c.setSize(5,5); c.setPos(2,3);
        c.visible=true; c.hitThreshold=0;
        var q=new Layer(w,null); q.setSize(100,100);
        "#,
    );
    for expression in [
        "p.getLayerAt(11,16) === b",
        "a.getLayerAt(1,1) === b", // Searches siblings, not only descendants.
        "b.getLayerAt(1,1,true) === a",
        "b.getLayerAt(2,3,true) === c", // Exclusion retains children.
        "c.getLayerAt(0,0,true) === b",
        "q.getLayerAt(11,16) === q", // Uses this tree's primary layer.
        "p.getLayerAt(90,90) === p",
        "p.getLayerAt(90,90,true) === null",
        "p.getLayerAt(-1,0) === null",
        "p.getLayerAt(100,0) === null",
        "(b.bringToBack(), p.getLayerAt(11,16) === a)",
        "(a.visible=false, p.getLayerAt(11,16) === b)",
        "(b.visible=false, p.getLayerAt(12,18) === p)",
    ] {
        assert_eq!(
            s.evaluate(expression).unwrap(),
            Value::Integer(1),
            "{expression}"
        );
    }
    assert!(s.evaluate("p.getLayerAt(1)").is_err());
}

#[test]
fn get_layer_at_disabled_layers_occlude_and_optional_flags_default_to_false() {
    let (_p, _s, mut s, _) = session(SETUP);
    for expression in [
        "a.getLayerAt(1,1,void,void) === a",
        "(a.enabled=false, p.getLayerAt(11,6) === null)",
        "p.getLayerAt(11,6,void,true) === a",
        "a.getLayerAt(1,1,true) === p",
        "(a.enabled=true, p.enabled=false, p.getLayerAt(11,6) === null)",
        "p.getLayerAt(11,6,false,true) === a",
        "(p.enabled=true, a.setMode(), p.getLayerAt(1,1) === null)",
        "p.getLayerAt(1,1,false,true) === p",
        "p.getLayerAt(11,6) === a",
    ] {
        assert_eq!(
            s.evaluate(expression).unwrap(),
            Value::Integer(1),
            "{expression}"
        );
    }
}

#[test]
fn get_layer_at_uses_masks_provinces_and_hit_callbacks() {
    let (_p, _s, mut s, _) = session(SETUP);
    for expression in [
        "(a.hitThreshold=16, p.getLayerAt(11,6) === p)",
        "(a.setMaskPixel(1,1,16), p.getLayerAt(11,6) === a)",
        "(a.setImageSize(22,20), a.imageLeft=-1, p.getLayerAt(10,6) === a)",
        "p.getLayerAt(11,6) === p",
        "(a.hitType=htProvince, p.getLayerAt(10,6) === p)",
        "(a.setProvincePixel(1,1,1), p.getLayerAt(10,6) === a)",
        "(a.onHitTest=function(x,y,hit){log.add(x+':'+y); (global.Layer.onHitTest incontextof this)(x,y,false);}, p.getLayerAt(10,6) === p)",
        "log[log.count-1] === '0:1'",
    ] {
        assert_eq!(
            s.evaluate(expression).unwrap(),
            Value::Integer(1),
            "{expression}"
        );
    }
}

#[test]
fn get_layer_at_preserves_outer_hit_results_and_allows_invalidation() {
    let (_p, _s, mut s, _) = session(SETUP);
    s.evaluate(
        r#"a.onHitTest=function(x,y,hit) {
        (global.Layer.onHitTest incontextof this)(x,y,false);
        global.p.getLayerAt(90,40);
    }"#,
    )
    .unwrap();
    assert_eq!(
        s.evaluate("p.getLayerAt(11,6) === p").unwrap(),
        Value::Integer(1)
    );
    s.evaluate(
        r#"(a.onHitTest=function(x,y,hit) {
        global.p.getLayerAt(90,40);
    }, p.onHitTest=function(x,y,hit) {
        (global.Layer.onHitTest incontextof this)(x,y,false);
    })"#,
    )
    .unwrap();
    assert_eq!(
        s.evaluate("p.getLayerAt(11,6) === a").unwrap(),
        Value::Integer(1)
    );
    s.evaluate("a.onHitTest=function(){invalidate global.a;}")
        .unwrap();
    assert_eq!(s.evaluate("p.getLayerAt(11,6)").unwrap(), Value::NULL);
    s.evaluate("p.onHitTest=function(){invalidate global.w;}")
        .unwrap();
    assert_eq!(s.evaluate("p.getLayerAt(1,1)").unwrap(), Value::NULL);
}
