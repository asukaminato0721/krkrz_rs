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
fn original_layer_corpus() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/layer.json")).unwrap();
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
fn hierarchy_rejects_cycles_and_cross_tree_parents_without_mutation() {
    let (_project, _saves, mut session) = session(
        "var w=new Window(),p=new Layer(w,null),q=new Layer(w,null),a=new Layer(w,p),b=new Layer(w,a);",
        100_000,
    );
    for code in ["a.parent=b", "a.parent=a", "a.parent=q"] {
        assert!(session.evaluate(code).is_err(), "{code}");
        assert_eq!(
            session
                .evaluate("a.parent===p && b.parent===a && p.children.count==1")
                .unwrap(),
            Value::Integer(1)
        );
    }
    session.evaluate("invalidate a").unwrap();
    assert_eq!(
        session
            .evaluate("isvalid b && b.parent===null && p.children.count==0")
            .unwrap(),
        Value::Integer(1)
    );
    // A severed child is not a primary layer and can still be positioned.
    session.evaluate("b.setPos(3,4)").unwrap();
    assert_eq!(session.evaluate("b.left+b.top").unwrap(), Value::Integer(7));
}
#[test]
fn surface_resize_preserves_pixels_and_enforces_allocation_limits() {
    let (_project, _saves, mut session) = session(
        "var w=new Window(),p=new Layer(w,null),a=new Layer(w,p);a.setImageSize(2,2);a.fillRect(0,0,2,2,0x80345678);a.setImageSize(4,3);",
        100_000,
    );
    assert_eq!(session.evaluate("[a.getMainPixel(1,1),a.getMaskPixel(1,1),a.getMainPixel(3,2),a.getMaskPixel(3,2)].join(',')").unwrap(), Value::string("3430008,128,16777215,0"));
    let error = session
        .evaluate("a.setImageSize(2147483647,2147483647)")
        .unwrap_err();
    assert!(error.downcast_ref::<krkrz_tjs::VmAbort>().is_some());
    assert_eq!(
        session.evaluate("a.imageWidth*10+a.imageHeight").unwrap(),
        Value::Integer(43)
    );
    assert!(session.evaluate("p.left=1").is_err());
    assert!(session.evaluate("a.children=[]").is_err());
    // A clipped write must not affect the underlying pixels.
    session
        .evaluate("(a.setClip(0,0,1,1),a.setMainPixel(1,1,0))")
        .unwrap();
    assert_eq!(
        session.evaluate("a.getMainPixel(1,1)").unwrap(),
        Value::Integer(0x345678)
    );
}
