use krkrz_runtime::Session;
use krkrz_tjs::Value;
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    name: String,
    source: String,
    expected: Value,
}

fn session() -> (tempfile::TempDir, tempfile::TempDir, Session) {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    for entry in std::fs::read_dir(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/images"
    ))
    .unwrap()
    {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), project.path().join(entry.file_name())).unwrap();
    }
    let session = Session::open(project.path(), Some(saves.path()), None, 1_000_000).unwrap();
    (project, saves, session)
}

#[test]
fn original_image_path_and_paint_corpus() {
    for fixture in [
        include_str!("fixtures/layer_images.json"),
        include_str!("fixtures/assign_images.json"),
        include_str!("fixtures/layer_blit.json"),
        include_str!("fixtures/layer_paint.json"),
        include_str!("fixtures/storage_paths.json"),
    ] {
        for case in serde_json::from_str::<Vec<Case>>(fixture).unwrap() {
            let (project, _saves, mut session) = session();
            std::fs::write(project.path().join("case.tjs"), case.source).unwrap();
            assert_eq!(
                session
                    .execute_storage("case.tjs")
                    .unwrap_or_else(|e| panic!("{}: {e:#}", case.name)),
                case.expected,
                "{}",
                case.name
            );
        }
    }
}

#[test]
fn paint_callbacks_can_invalidate_and_request_another_frame() {
    let (project, _saves, mut session) = session();
    std::fs::write(project.path().join("case.tjs"), r#"
global.w=new Window();global.p=new Layer(w,null);global.a=new Layer(w,p);global.b=new Layer(w,p);global.events=[];
p.onPaint=function(){events.add('parent');invalidate a;};
a.onPaint=function(){events.add('invalid child');};
b.onPaint=function(){events.add('child');b.update();};
p.update();a.update();b.update();
"#).unwrap();
    session.execute_storage("case.tjs").unwrap();
    let window = session.evaluate("w").unwrap();
    session.prepare_window_paint(&window).unwrap();
    assert_eq!(
        session.evaluate("events.join(',')").unwrap(),
        Value::string("parent,child")
    );
    assert_eq!(
        session.evaluate("b.callOnPaint").unwrap(),
        Value::Integer(1)
    );
    session.prepare_window_paint(&window).unwrap();
    assert_eq!(
        session.evaluate("events.join(',')").unwrap(),
        Value::string("parent,child,child")
    );
}

#[test]
fn cached_images_are_independent_and_cache_limits_apply() {
    let (project, _saves, mut session) = session();
    std::fs::write(
        project.path().join("case.tjs"),
        r#"
System.graphicCacheLimit=1024;
global.w=new Window();global.a=new Layer(w,null);global.b=new Layer(w,a);
a.loadImages('pixels.png');a.fillRect(0,0,4,1,0);
b.loadImages('pixels.png');
return b.getMainPixel(0,0);
"#,
    )
    .unwrap();
    assert_eq!(
        session.execute_storage("case.tjs").unwrap(),
        Value::Integer(0xff0000)
    );
    assert!(
        session
            .services
            .image_cache
            .get(&project.path().join("pixels.png").to_string_lossy())
            .is_some()
    );
    session.evaluate("System.doCompact(10)").unwrap();
    assert!(
        session
            .services
            .image_cache
            .get(&project.path().join("pixels.png").to_string_lossy())
            .is_some()
    );
    session.evaluate("System.doCompact(15)").unwrap();
    assert!(
        session
            .services
            .image_cache
            .get(&project.path().join("pixels.png").to_string_lossy())
            .is_none()
    );
    assert_eq!(
        session.evaluate("b.getMainPixel(0,0)").unwrap(),
        Value::Integer(0xff0000)
    );
    session.evaluate("b.loadImages('pixels.png')").unwrap();
    session.evaluate("System.graphicCacheLimit=0").unwrap();
    assert!(
        session
            .services
            .image_cache
            .get(&project.path().join("pixels.png").to_string_lossy())
            .is_none()
    );
}
