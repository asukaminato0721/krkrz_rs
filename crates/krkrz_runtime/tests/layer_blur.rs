use krkrz_runtime::Session;
use krkrz_tjs::Value;
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    name: String,
    source: String,
}

#[test]
fn library_box_blur_pixels() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/layer_blur.json")).unwrap();
    let pixels: std::collections::BTreeMap<String, String> =
        serde_json::from_str(include_str!("fixtures/library_raster.json")).unwrap();
    for case in cases {
        let project = tempfile::tempdir().unwrap();
        let saves = tempfile::tempdir().unwrap();
        std::fs::write(project.path().join("case.tjs"), case.source).unwrap();
        let mut session = Session::open(project.path(), Some(saves.path()), None, 100_000).unwrap();
        assert_eq!(
            session
                .execute_storage("case.tjs")
                .unwrap_or_else(|error| panic!("{}: {error:#}", case.name)),
            Value::string(&pixels[&case.name]),
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
    let mut session = Session::open(project.path(), Some(saves.path()), None, 100_000).unwrap();
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

#[test]
fn box_blur_averages_only_the_available_neighborhood() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), None, 100_000).unwrap();
    let source = "var w=new Window(),l=new Layer(w,null);l.setImageSize(3,1);l.face=dfOpaque;l.fillRect(0,0,3,1,0xff000000);l.setMainPixel(1,0,0xffffff);l.doBoxBlur(1,0);return [l.getMainPixel(0,0),l.getMainPixel(1,0),l.getMainPixel(2,0)].join(',');";
    let value = session
        .vm
        .execute(
            &krkrz_tjs::compile("blur", source).unwrap(),
            &mut session.services,
            &mut session.budget,
        )
        .unwrap();
    assert_eq!(value, Value::string("8421504,5592405,8421504"));
}
