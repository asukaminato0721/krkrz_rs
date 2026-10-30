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
fn script_cursor_moves_update_hover_after_focus_and_use_client_coordinates() {
    let (_project, _saves, mut s, w) = session(
        r#"
        var log=[],w=new Window(),p=new Layer(w,null),a=new Layer(w,p);
        w.visible=true;w.setInnerSize(200,100);w.setZoom(2,1);
        p.setSize(100,50);p.visible=true;
        a.setSize(20,20);a.setPos(10,5);a.visible=a.focusable=true;a.hitThreshold=0;
        a.onFocus=function(){log.add('focus');cursorX=2;cursorY=3;};
        a.onMouseEnter=function(){log.add('enter:'+focused);focus();opacity=128;};
        a.onMouseMove=function(x,y){log.add('move:'+x+','+y);};
        a.onMouseLeave=function(){log.add('leave');opacity=255;};
    "#,
    );
    s.evaluate("a.focus()").unwrap();
    assert_eq!(s.evaluate("log.join('|')").unwrap(), Value::string("focus"));
    s.dispatch_events().unwrap();
    assert_eq!(
        s.evaluate("log.join('|')").unwrap(),
        Value::string("focus|enter:1|move:2,3")
    );
    assert_eq!(s.evaluate("a.opacity").unwrap(), Value::Integer(128));
    assert_eq!(s.take_cursor_position(&w).unwrap(), Some([24, 16]));
    assert_eq!(s.take_cursor_position(&w).unwrap(), None);
    // Coalesce cursor writes. Intermediate points must not produce hover events.
    s.evaluate("a.setCursorPos(18,18)").unwrap();
    s.evaluate("p.setCursorPos(80,40)").unwrap();
    s.dispatch_events().unwrap();
    assert_eq!(
        s.evaluate("log.join('|')").unwrap(),
        Value::string("focus|enter:1|move:2,3|leave")
    );
    assert_eq!(s.evaluate("a.opacity").unwrap(), Value::Integer(255));
    assert_eq!(s.take_cursor_position(&w).unwrap(), Some([160, 80]));
    // A newer physical move cancels an unprocessed script cursor request.
    s.evaluate("a.setCursorPos(2,3)").unwrap();
    s.input(
        &w,
        InputEvent::PointerMove {
            x: 2,
            y: 2,
            shift: 0,
        },
    )
    .unwrap();
    s.dispatch_events().unwrap();
    assert_eq!(s.take_cursor_position(&w).unwrap(), None);
    assert_eq!(s.evaluate("a.opacity").unwrap(), Value::Integer(255));
}

#[test]
fn image_cursors_are_cached_inherited_and_preserved_after_load_failure() {
    use image::ImageEncoder;
    let (project, _save, mut s, window) = session(SETUP);
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            &[255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 0, 0, 0, 0],
            2,
            2,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
    let mut cur = vec![0, 0, 2, 0, 1, 0, 2, 2, 0, 0, 1, 0, 1, 0];
    cur.extend_from_slice(&(png.len() as u32).to_le_bytes());
    cur.extend_from_slice(&22u32.to_le_bytes());
    cur.extend(png);
    std::fs::write(project.path().join("pointer.cur"), cur).unwrap();
    std::fs::write(project.path().join("broken.cur"), b"bad cursor").unwrap();
    s.evaluate("p.cursor='pointer.cur'").unwrap();
    let handle = s.evaluate("p.cursor").unwrap().integer().unwrap() as i32;
    assert_eq!(handle, 2);
    let image = s.cursor_image(handle).unwrap();
    assert_eq!(image.hotspot(), [1, 1]);
    assert_eq!((image.image().width, image.image().height), (2, 2));
    s.input(
        &window,
        InputEvent::PointerMove {
            x: 24,
            y: 14,
            shift: 0,
        },
    )
    .unwrap();
    assert_eq!(s.window_cursor(&window).unwrap(), handle);
    s.evaluate("a.cursor=crHandPoint").unwrap();
    assert_eq!(s.window_cursor(&window).unwrap(), -21);
    s.evaluate("a.cursor='POINTER.CUR'").unwrap();
    assert_eq!(
        s.evaluate("a.cursor").unwrap(),
        Value::Integer(handle.into())
    );
    assert!(s.evaluate("a.cursor='broken.cur'").is_err());
    assert_eq!(s.window_cursor(&window).unwrap(), handle);
    s.evaluate("a.cursor=crDefault").unwrap();
    assert_eq!(s.window_cursor(&window).unwrap(), handle);
    s.evaluate("p.cursor=crNone").unwrap();
    assert_eq!(s.window_cursor(&window).unwrap(), -1);
    s.evaluate("a.cursor=crHBeam").unwrap();
    assert_eq!(s.window_cursor(&window).unwrap(), 1);
    assert!(s.cursor_image(1).is_none());
}

#[test]
fn legacy_quake_moves_presentation_and_hit_testing_then_restores_origin() {
    let (_project, _saves, mut s, w) = session(
        r#"
        class QuakeWindow extends Window {
            function QuakeWindow(){super.Window();visible=true;setInnerSize(200,100);}
            function stopQuake(){setLayerPos(0,0);}
        }
        var w=new QuakeWindow(),p=new Layer(w,null),a=new Layer(w,p),log=[];
        p.setSize(100,50);a.setSize(20,20);a.setPos(10,5);
        a.visible=true;a.hitThreshold=0;
        a.onMouseMove=function(x,y){log.add(x+','+y);};
        w.setPos(30,40);w.setZoom(2,1);
    "#,
    );
    let Value::Object(reference) = &w else {
        panic!("expected Window")
    };
    let id = reference.object.unwrap();
    let mut frame = krkrz_runtime::WindowFrameState::default();
    assert!(
        s.capture_window_if_changed(&w, &mut frame)
            .unwrap()
            .is_some()
    );
    s.evaluate("w.setLayerPos(-3,4)").unwrap();
    assert_eq!(
        s.services.windows[&id].draw_rect(100, 50),
        [-6, 8, 200, 100]
    );
    assert!(
        s.capture_window_if_changed(&w, &mut frame)
            .unwrap()
            .is_some()
    );
    assert!(
        s.capture_window_if_changed(&w, &mut frame)
            .unwrap()
            .is_none()
    );
    s.input(
        &w,
        InputEvent::PointerMove {
            x: 24,
            y: 28,
            shift: 0,
        },
    )
    .unwrap();
    assert_eq!(s.evaluate("log.join('|')").unwrap(), Value::string("5,5"));
    assert_eq!(
        s.evaluate("[w.layerLeft,w.layerTop,w.left,w.top,p.left,p.top].join(',')")
            .unwrap(),
        Value::string("-3,4,30,40,0,0")
    );
    // Fractional zoom truncates negative offsets toward zero.
    s.evaluate("w.setZoom(1,2)").unwrap();
    assert_eq!(s.services.windows[&id].draw_rect(100, 50), [-1, 2, 50, 25]);
    // Resizing and full screen apply the same offset to the fitted canvas.
    s.services.windows.get_mut(&id).unwrap().fit_to_client = true;
    s.resize_window(&w, 300, 100).unwrap();
    assert_eq!(
        s.services.windows[&id].draw_rect(100, 50),
        [44, 8, 200, 100]
    );
    s.evaluate("w.layerLeft=2").unwrap();
    s.evaluate("w.layerTop=-1").unwrap();
    assert_eq!(
        s.services.windows[&id].draw_rect(100, 50),
        [54, -2, 200, 100]
    );
    s.evaluate("w.fullScreen=true").unwrap();
    let shifted = s.services.windows[&id].draw_rect(100, 50);
    s.evaluate("w.stopQuake()").unwrap();
    let restored = s.services.windows[&id].draw_rect(100, 50);
    assert_eq!(shifted[0] - restored[0], restored[2] * 2 / 100);
    assert_eq!(shifted[1] - restored[1], -restored[3] / 50);
    s.evaluate("w.fullScreen=false").unwrap();
    assert_eq!(
        s.services.windows[&id].draw_rect(100, 50),
        [50, 0, 200, 100]
    );
    // The legacy full-screen path also scales offsets without fit_to_client.
    s.services.windows.get_mut(&id).unwrap().fit_to_client = false;
    s.evaluate("w.fullScreen=true").unwrap();
    let origin = s.services.windows[&id].draw_rect(100, 50);
    s.evaluate("w.setLayerPos(3,-3)").unwrap();
    let shifted = s.services.windows[&id].draw_rect(100, 50);
    assert!(shifted[0] > origin[0] && shifted[1] < origin[1]);
    s.evaluate("w.stopQuake()").unwrap();
    assert_eq!(s.services.windows[&id].draw_rect(100, 50), origin);
}

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
fn pad_queries_share_edges_and_repeats_do_not_create_new_presses() {
    use krkrz_runtime::gamepad::PadKey;
    let (_project, _saves, mut s, w) = session(SETUP);
    s.evaluate("w.onKeyDown=function(key,shift){global.log.add(key+':'+shift+':'+System.getKeyState(VK_PADANY));}").unwrap();
    let down = |key: PadKey, shift| InputEvent::KeyDown {
        key: key.key_code(),
        shift,
    };
    let up = |key: PadKey| InputEvent::KeyUp {
        key: key.key_code(),
        shift: 0,
    };
    s.input(&w, down(PadKey::Button1, 0)).unwrap();
    // Current queries must not consume a pad's pending press.
    for expression in [
        "System.getKeyState(VK_PAD1)",
        "System.getKeyState(VK_PADANY)",
        "System.getKeyState(VK_PAD1,false)",
        "!System.getKeyState(VK_PADANY,false)",
    ] {
        assert_eq!(
            s.evaluate(expression).unwrap(),
            Value::Integer(1),
            "{expression}"
        );
    }
    s.input(&w, down(PadKey::Button1, 128)).unwrap();
    assert_eq!(
        s.evaluate("System.getKeyState(VK_PADANY,false)").unwrap(),
        Value::Integer(0)
    );
    s.input(&w, down(PadKey::Left, 0)).unwrap();
    s.input(&w, up(PadKey::Left)).unwrap();
    for expression in [
        "!System.getKeyState(VK_PADLEFT)",
        "System.getKeyState(VK_PADANY)",
        "System.getKeyState(VK_PADANY,false)",
        "!System.getKeyState(VK_PADLEFT,false)",
    ] {
        assert_eq!(
            s.evaluate(expression).unwrap(),
            Value::Integer(1),
            "{expression}"
        );
    }
    assert_eq!(
        s.evaluate("log[0]+'|'+log[1]").unwrap(),
        Value::string("448:0:1|448:128:1")
    );
    s.input(&w, InputEvent::Focus(false)).unwrap();
    s.input(&w, down(PadKey::Button2, 0)).unwrap();
    s.input(&w, up(PadKey::Button2)).unwrap();
    s.input(&w, InputEvent::KeyDown { key: 65, shift: 0 })
        .unwrap();
    // A keyboard key cannot make PADANY true. A short pad tap remains observable.
    for expression in [
        "!System.getKeyState(VK_PADANY)",
        "System.getKeyState(VK_PADANY,false)",
        "!System.getKeyState(VK_PADANY,false)",
        "System.getKeyState(65,false)",
    ] {
        assert_eq!(
            s.evaluate(expression).unwrap(),
            Value::Integer(1),
            "{expression}"
        );
    }
}

#[test]
fn script_pad_mapping_posts_ordered_keys_without_changing_physical_key_state() {
    use krkrz_runtime::gamepad::PadKey;
    let (_project, _saves, mut s, w) = session(
        r#"
        var log=[];
        class TestWindow extends Window {
            function TestWindow(){super.Window();visible=true;}
            function action(e){
                log.add(e.type+':'+e.key);
                if(e.key==VK_PAD1)
                    postInputEvent(e.type,%[key:VK_RETURN,shift:e.shift]);
            }
        }
        var w=new TestWindow();
    "#,
    );
    s.input(
        &w,
        InputEvent::KeyDown {
            key: PadKey::Button1.key_code(),
            shift: 0,
        },
    )
    .unwrap();
    s.input(
        &w,
        InputEvent::KeyUp {
            key: PadKey::Button1.key_code(),
            shift: 0,
        },
    )
    .unwrap();
    assert_eq!(
        s.evaluate("log.join('|')").unwrap(),
        Value::string("onKeyDown:448|onKeyUp:448")
    );
    s.dispatch_events().unwrap();
    assert_eq!(
        s.evaluate("log.join('|')").unwrap(),
        Value::string("onKeyDown:448|onKeyUp:448|onKeyDown:13|onKeyUp:13")
    );
    assert_eq!(
        s.evaluate("System.getKeyState(VK_RETURN,false)").unwrap(),
        Value::Integer(0)
    );
    assert_eq!(
        s.evaluate("System.getKeyState(VK_PADANY,false)").unwrap(),
        Value::Integer(1)
    );
    for expression in [
        "w.postInputEvent('onMouseDown',%[key:1,shift:0])",
        "w.postInputEvent('onKeyDown')",
        "w.postInputEvent('onKeyDown',%[key:13])",
        "w.postInputEvent('onKeyPress',%[]) ",
    ] {
        assert!(s.evaluate(expression).is_err(), "{expression}");
    }
    s.evaluate("w.postInputEvent('onKeyPress',%[key:65])")
        .unwrap();
    s.dispatch_events().unwrap();
    assert_eq!(
        s.evaluate("log[log.count-1]").unwrap(),
        Value::string("onKeyPress:A")
    );
    s.evaluate("w.postInputEvent('onKeyDown',%[key:13,shift:0])")
        .unwrap();
    s.evaluate("invalidate w").unwrap();
    s.dispatch_events().unwrap();
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
