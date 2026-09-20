use krkrz_runtime::Session;
use krkrz_tjs::Value;
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    name: String,
    source: String,
    expected: Value,
}

#[test]
fn original_box_blur_pixels() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/layer_blur.json")).unwrap();
    for case in cases {
        let project = tempfile::tempdir().unwrap();
        let saves = tempfile::tempdir().unwrap();
        std::fs::write(project.path().join("case.tjs"), case.source).unwrap();
        let mut session =
            Session::open(project.path(), Some(saves.path()), false, 100_000).unwrap();
        assert_eq!(
            session
                .execute_storage("case.tjs")
                .unwrap_or_else(|error| panic!("{}: {error:#}", case.name)),
            case.expected,
            "{}",
            case.name,
        );
    }
}

#[test]
fn blur_bounds_large_radii_and_handles_short_images_deterministically() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("case.tjs"),
        r#"
        var w=new Window(), p=new Layer(w,null), l=new Layer(w,p);
        l.setImageSize(1,1); l.face=dfOpaque;
        l.fillRect(0,0,1,1,0x80345678);
        l.doBoxBlur(1,5);
        return l.getMainPixel(0,0)+":"+l.getMaskPixel(0,0);
    "#,
    )
    .unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), false, 100_000).unwrap();
    assert_eq!(
        session.execute_storage("case.tjs").unwrap(),
        Value::string("3430008:128")
    );
    for source in [
        "l.doBoxBlur(4096,4096)",
        "l.doBoxBlur(-2147483648,-2147483648)",
    ] {
        assert!(session.evaluate(source).is_err());
        assert_eq!(
            session.evaluate("l.getMainPixel(0,0)").unwrap(),
            Value::Integer(0x345678)
        );
    }
}
