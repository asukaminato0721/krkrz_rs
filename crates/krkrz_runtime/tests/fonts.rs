use krkrz_runtime::{Session, fonts::FontBook};
use krkrz_tjs::Value;

const FONT: &[u8] = include_bytes!("fixtures/synthetic.ttf");
#[test]
fn font_names_deduplication_metrics_and_coverage() {
    let mut book = FontBook::default();
    assert_eq!(book.add(FONT.to_vec()).unwrap(), 1);
    assert_eq!(book.add(FONT.to_vec()).unwrap(), 1);
    assert_eq!(book.face_count(), 1);
    let glyph = book.rasterize("Kirikiri Synthetic", 'A', 20.0).unwrap();
    assert_eq!(
        (glyph.left, glyph.top, glyph.width, glyph.height),
        (2, -14, 10, 14)
    );
    assert_eq!(glyph.advance, 14.0);
    let area = glyph
        .coverage
        .iter()
        .map(|v| *v as f32 / 255.0)
        .sum::<f32>();
    assert!((area - 70.0).abs() < 1.0, "triangle area: {area}");
    assert_eq!(
        glyph,
        book.rasterize("Kirikiri Synthetic Regular", 'あ', 20.0)
            .unwrap()
    );
    let space = book.rasterize("Kirikiri Synthetic", ' ', 20.0).unwrap();
    assert_eq!(space.advance, 14.0);
    assert!(space.coverage.is_empty());
}
#[test]
fn malformed_fonts_and_glyph_limits() {
    let mut book = FontBook::default();
    for data in [Vec::new(), b"bad font".to_vec(), FONT[..80].to_vec()] {
        assert_eq!(book.add(data).unwrap(), 0);
    }
    assert_eq!(book.face_count(), 0);
    assert!(book.rasterize("missing font", 'A', 20.0).is_err());
    book.add(FONT.to_vec()).unwrap();
    for size in [0.0, -1.0, f32::NAN, f32::INFINITY, 5000.0] {
        assert!(book.rasterize("Kirikiri Synthetic", 'A', size).is_err());
    }
    assert_eq!(
        book.rasterize("missing Windows font", 'A', 20.0).unwrap(),
        book.rasterize("Kirikiri Synthetic", 'A', 20.0).unwrap()
    );
}

#[test]
fn unavailable_saved_face_uses_one_fallback_for_layout_and_drawing() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("synthetic.ttf"), FONT).unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), false, 100_000).unwrap();
    let source = r#"
Plugins.link('PackinOne.dll');System.addFont('synthetic.ttf',false);
var w=new Window(),l=new Layer(w,null),f=l.font;
l.setSize(32,28);l.face=dfAlpha;f.height=20;f.face='Kirikiri Synthetic';
var width=f.getTextWidth('Aあ');
l.fillRect(0,0,32,28,0);l.drawText(2,1,'Aあ',0x80c040);
var pixels=[];for(var y=0;y<28;y++)for(var x=0;x<32;x++){pixels.add(l.getMainPixel(x,y));pixels.add(l.getMaskPixel(x,y));}
f.face='尮僲妏僑僔僢僋B';
l.fillRect(0,0,32,28,0);l.drawText(2,1,'Aあ',0x80c040);
var i=0;for(var y=0;y<28;y++)for(var x=0;x<32;x++){if(pixels[i++]!=l.getMainPixel(x,y) || pixels[i++]!=l.getMaskPixel(x,y))return false;}
return width==f.getTextWidth('Aあ');
"#;
    let program = krkrz_tjs::compile("fallback", source).unwrap();
    assert_eq!(
        session
            .vm
            .execute(&program, &mut session.services, &mut session.budget)
            .unwrap(),
        Value::Integer(1)
    );
}
#[test]
fn original_add_font_api_registers_private_font_data() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("synthetic.ttf"), FONT).unwrap();
    std::fs::write(project.path().join("invalid.ttf"), b"not a font").unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), false, 100_000).unwrap();
    let program = krkrz_tjs::compile("fonts", "Plugins.link('PackinOne.dll');return [typeof System.addFont('missing.ttf'),System.addFont('invalid.ttf',false),System.addFont('synthetic.ttf',false),System.addFont('synthetic.ttf',false)].join('|');").unwrap();
    let result = session
        .vm
        .execute(&program, &mut session.services, &mut session.budget)
        .unwrap();
    assert_eq!(result, Value::string("void|||"));
    assert_eq!(session.services.fonts.face_count(), 1);
    assert!(
        !session
            .services
            .fonts
            .rasterize("Kirikiri Synthetic", 'A', 20.0)
            .unwrap()
            .coverage
            .is_empty()
    );
}

#[test]
fn original_font_measurement_corpus() {
    #[derive(serde::Deserialize)]
    struct Case {
        name: String,
        source: String,
        expected: Value,
    }
    let cases: Vec<Case> = [
        include_str!("fixtures/font_metrics.json"),
        include_str!("fixtures/layer_text.json"),
    ]
    .into_iter()
    .flat_map(|fixture| serde_json::from_str::<Vec<Case>>(fixture).unwrap())
    .collect();
    for case in cases {
        let project = tempfile::tempdir().unwrap();
        let saves = tempfile::tempdir().unwrap();
        std::fs::write(project.path().join("synthetic.ttf"), FONT).unwrap();
        std::fs::write(project.path().join("case.tjs"), case.source).unwrap();
        let mut session =
            Session::open(project.path(), Some(saves.path()), false, 100_000).unwrap();
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
