use krkrz_runtime::{Session, WindowFrameState};
use krkrz_tjs::Value;

#[test]
fn layered_window_displays_submitted_rows_and_owns_its_snapshot() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), None, 100_000).unwrap();
    std::fs::write(
        project.path().join("layered.tjs"),
        r#"
        Plugins.link('layeredwindow.dll');
        global.w=new Window();var p=new Layer(w,null);p.setSize(2,2);
        global.l=new Layer(w,p);l.setImageSize(2,2);l.fillRect(0,0,2,2,0xff112233);
        l.setMaskPixel(1,0,64);l.setMainPixel(0,1,0x445566);l.flipUD();
        layeredwindow(w.HWND,l.mainImageBuffer,l.mainImageBufferPitch,2,2);
    "#,
    )
    .unwrap();
    session.execute_storage("layered.tjs").unwrap();
    let window = session.evaluate("w").unwrap();
    let mut state = WindowFrameState::default();
    let frame = session
        .capture_window_if_changed(&window, &mut state)
        .unwrap()
        .unwrap();
    assert_eq!(
        frame.rgba,
        [
            0x11, 0x22, 0x33, 255, 0x11, 0x22, 0x33, 64, 0x44, 0x55, 0x66, 255, 0x11, 0x22, 0x33,
            255
        ]
    );
    session.evaluate("l.fillRect(0,0,2,2,0xff000000)").unwrap();
    assert_eq!(session.capture_window(&window).unwrap().rgba, frame.rgba);
    session
        .evaluate("layeredwindow(w.HWND,l.mainImageBuffer,l.mainImageBufferPitch,2,2)")
        .unwrap();
    assert_eq!(
        session
            .capture_window_if_changed(&window, &mut state)
            .unwrap()
            .unwrap()
            .rgba,
        [0, 0, 0, 255].repeat(4)
    );
    session.evaluate("invalidate l").unwrap();
    assert_eq!(
        session.capture_window(&window).unwrap().rgba,
        [0, 0, 0, 255].repeat(4)
    );
}

#[test]
fn layered_window_rejects_invalid_handles_pitch_and_dimensions() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), None, 10_000).unwrap();
    std::fs::write(
        project.path().join("layered.tjs"),
        r#"
        Plugins.link('layeredwindow.dll');
        var w=new Window(),p=new Layer(w,null),l=new Layer(w,p),n=0;
        l.setImageSize(2,2);
        try{layeredwindow(123,l.mainImageBuffer,-8,2,2);}catch(e){n++;}
        try{layeredwindow(w.HWND,123,-8,2,2);}catch(e){n++;}
        try{layeredwindow(w.HWND,l.mainImageBuffer,8,2,2);}catch(e){n++;}
        try{layeredwindow(w.HWND,l.mainImageBuffer,-8,3,2);}catch(e){n++;}
        var buffer=l.mainImageBuffer;invalidate l;
        try{layeredwindow(w.HWND,buffer,-8,2,2);}catch(e){n++;}
        return n;
    "#,
    )
    .unwrap();
    assert_eq!(
        session.execute_storage("layered.tjs").unwrap(),
        Value::Integer(5)
    );
}
