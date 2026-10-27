use krkrz_runtime::{Session, fonts::FontBook};
use krkrz_tjs::Value;

const FONT: &[u8] = include_bytes!("fixtures/synthetic.ttf");

fn prerendered_font(version: u8) -> Vec<u8> {
    let mut data = b"TVP pre-rendered font\x1a".to_vec();
    data.extend([version, 2]);
    for value in [1u32, 36, 38] {
        data.extend(value.to_le_bytes());
    }
    data.extend(65u16.to_le_bytes());
    data.extend(58u32.to_le_bytes());
    // A 2x2 glyph at baseline (1,-3), with advance 9 and coverage [64,32,32,32].
    for value in [2i16, 2, 1, 3, 9, 0, 9, 0] {
        data.extend(value.to_le_bytes());
    }
    data.extend(if version == 0 {
        vec![64, 32, 65, 2]
    } else {
        vec![64, 32, 66]
    });
    data
}

#[test]
fn prerendered_mapping_controls_shared_metrics_pixels_and_unmapping() {
    for version in [0, 1] {
        let project = tempfile::tempdir().unwrap();
        let saves = tempfile::tempdir().unwrap();
        std::fs::write(project.path().join("synthetic.ttf"), FONT).unwrap();
        std::fs::write(project.path().join("font.tft"), prerendered_font(version)).unwrap();
        std::fs::write(project.path().join("invalid.tft"), b"invalid").unwrap();
        let mut session = Session::open(project.path(), Some(saves.path()), None, 100_000).unwrap();
        let source = r#"
Plugins.link('PackinOne.dll');System.addFont('synthetic.ttf',false);
var w=new Window(),l=new Layer(w,null),other=new Layer(w,l),f=l.font,g=other.font;
l.setSize(32,28);l.face=dfAlpha;l.fillRect(0,0,32,28,0);
f.face=g.face='Kirikiri Synthetic';f.height=g.height=20;
f.mapPrerenderedFont('font.tft');
var results=[f.getTextWidth('AA'),g.getTextWidth('Aあ'),f.getTextHeight('AA'),f.getEscWidthX('AA')];
// The mapped glyph stays antialiased even when drawText requests monochrome.
l.drawText(0,0,'AA',0xffffff,255,false);
var pixels=0,alpha=0;for(var y=0;y<28;y++)for(var x=0;x<32;x++){var a=l.getMaskPixel(x,y);if(a)pixels++;alpha+=a;}
results.add(pixels);results.add(alpha);
// The complete font style selects the mapping.
g.bold=true;results.add(g.getTextWidth('A'));g.bold=false;
g.height=21;results.add(g.getTextWidth('A'));g.height=20;
g.angle=900;results.add(g.getTextWidth('A'));g.angle=0;
g.face='other';results.add(g.getTextWidth('A'));g.face=f.face;
try {f.mapPrerenderedFont('invalid.tft');}catch(e){}
results.add(g.getTextWidth('A'));
// Remove the mapping through a different Font instance.
g.unmapPrerenderedFont();results.add(f.getTextWidth('A'));
g.unmapPrerenderedFont();
return results.join('|');
"#;
        let program = krkrz_tjs::compile("prerendered", source).unwrap();
        let result = session
            .vm
            .execute(&program, &mut session.services, &mut session.budget)
            .unwrap();
        assert_eq!(result, Value::string("18|23|20|18|8|1278|15|15|14|14|9|14"));
    }
}

#[test]
fn prerendered_fonts_reject_invalid_headers_and_compressed_runs() {
    let valid = prerendered_font(1);
    let mut cases = Vec::new();
    for length in [0, 21, 35, 37, 57] {
        cases.push((valid[..length].to_vec(), false));
    }
    for (offset, value) in [(0, 0), (22, 2), (23, 1), (28, 255), (32, 255), (38, 255)] {
        let mut data = valid.clone();
        data[offset] = value;
        cases.push((data, false));
    }
    for bytes in [vec![67], vec![64, 68], vec![64]] {
        let mut data = valid[..58].to_vec();
        data.extend(bytes);
        cases.push((data, true));
    }
    // Version 0 permits 0x41 followed by a count, but no other coverage >64.
    for bytes in [vec![64, 66], vec![64, 65], vec![64, 65, 4]] {
        let mut data = valid[..58].to_vec();
        data[22] = 0;
        data.extend(bytes);
        cases.push((data, true));
    }
    for (data, draw) in cases {
        let project = tempfile::tempdir().unwrap();
        let saves = tempfile::tempdir().unwrap();
        std::fs::write(project.path().join("synthetic.ttf"), FONT).unwrap();
        std::fs::write(project.path().join("font.tft"), data).unwrap();
        let mut session = Session::open(project.path(), Some(saves.path()), None, 100_000).unwrap();
        let source = format!(
            "Plugins.link('PackinOne.dll');System.addFont('synthetic.ttf',false);var w=new Window(),l=new Layer(w,null);l.setSize(32,28);l.font.height=20;l.font.mapPrerenderedFont('font.tft');{}",
            if draw {
                "l.drawText(0,0,'A',0xffffff);"
            } else {
                ""
            }
        );
        let program = krkrz_tjs::compile("malformed TFT", &source).unwrap();
        let error = session
            .vm
            .execute(&program, &mut session.services, &mut session.budget)
            .unwrap_err();
        assert!(format!("{error:#}").contains("prerendered"), "{error:#}");
    }
}
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
    let mut session = Session::open(project.path(), Some(saves.path()), None, 100_000).unwrap();
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
#[ignore = "requires an installed system CJK font"]
fn system_cjk_fallback_measures_and_draws_without_game_fonts() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), None, 100_000).unwrap();
    assert_eq!(session.services.fonts.face_count(), 0);
    session.evaluate("Scripts.exec('global.w=new Window();global.l=new Layer(w,null);l.setSize(400,60);l.face=dfAlpha;l.fillRect(0,0,400,60,0);l.font.face=\"黑体\";l.font.height=24;l.drawText(0,0,\"中文テストABC\",0xffffff);')").unwrap();
    assert!(
        session
            .evaluate("l.font.getTextWidth('中文テストABC')")
            .unwrap()
            .integer()
            .unwrap()
            > 0
    );
    let window = session.evaluate("w").unwrap();
    let image = session.capture_window(&window).unwrap();
    assert!(image.rgba.as_chunks::<4>().0.iter().any(|p| p[3] != 0));
    let book = &session.services.fonts;
    // Successful rendering must produce a Chinese glyph, not just .notdef.
    let chinese = book.rasterize("黑体", '黑', 24.0).unwrap();
    assert!(!chinese.coverage.is_empty());
    assert_ne!(chinese, book.rasterize("黑体", '\u{10ffff}', 24.0).unwrap());
    // System fallback does not masquerade as a game-registered private font.
    assert_eq!(book.face_count(), 0);
}
#[test]
fn original_add_font_api_registers_private_font_data() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("synthetic.ttf"), FONT).unwrap();
    std::fs::write(project.path().join("invalid.ttf"), b"not a font").unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), None, 100_000).unwrap();
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
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/font_metrics.json")).unwrap();
    for case in cases {
        let project = tempfile::tempdir().unwrap();
        let saves = tempfile::tempdir().unwrap();
        std::fs::write(project.path().join("synthetic.ttf"), FONT).unwrap();
        std::fs::write(project.path().join("case.tjs"), case.source).unwrap();
        let mut session = Session::open(project.path(), Some(saves.path()), None, 100_000).unwrap();
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

/// Keep the Windows reference pixels and hash, and explicitly constrain the
/// Fontations/Zeno differences. Do not rewrite the native golden on upgrades.
#[test]
fn fontations_text_pixels_and_quantified_original_differences() {
    #[derive(serde::Deserialize)]
    struct Original {
        name: String,
        source: String,
        expected: Value,
    }
    #[derive(serde::Deserialize)]
    struct Comparison {
        name: String,
        fontations_hash: u32,
        different_pixels: usize,
        max_channel_delta: u32,
        absolute_channel_error: u32,
        original_pixels_rle: Vec<[u32; 3]>,
    }
    fn hash(pixels: &[[u32; 2]]) -> u32 {
        pixels
            .iter()
            .flatten()
            .fold(2166136261u32, |h, v| (h ^ v).wrapping_mul(16777619))
    }
    let originals: Vec<Original> =
        serde_json::from_str(include_str!("fixtures/layer_text.json")).unwrap();
    let comparisons: Vec<Comparison> =
        serde_json::from_str(include_str!("fixtures/fontations_layer_text.json")).unwrap();
    assert_eq!(originals.len(), comparisons.len());
    for (original, comparison) in originals.into_iter().zip(comparisons) {
        assert_eq!(original.name, comparison.name);
        let native: Vec<[u32; 2]> = comparison
            .original_pixels_rle
            .iter()
            .flat_map(|[count, color, alpha]| {
                std::iter::repeat_n([*color, *alpha], *count as usize)
            })
            .collect();
        assert_eq!(native.len(), 32 * 28);
        assert_eq!(
            Value::Integer(hash(&native) as i64),
            original.expected,
            "native golden: {}",
            original.name
        );
        let project = tempfile::tempdir().unwrap();
        let saves = tempfile::tempdir().unwrap();
        std::fs::write(project.path().join("synthetic.ttf"), FONT).unwrap();
        let source = format!(
            "{}var pixels=[];for(var y=0;y<28;y++)for(var x=0;x<32;x++){{pixels.add(l.getMainPixel(x,y));pixels.add(l.getMaskPixel(x,y));}}return pixels.join(',');",
            original.source.split_once("var h=").unwrap().0
        );
        std::fs::write(project.path().join("case.tjs"), source).unwrap();
        let mut session = Session::open(project.path(), Some(saves.path()), None, 100_000).unwrap();
        let output = session.execute_storage("case.tjs").unwrap().text();
        let values: Vec<u32> = output.split(',').map(|s| s.parse().unwrap()).collect();
        let actual: Vec<[u32; 2]> = values.as_chunks::<2>().0.to_vec();
        assert_eq!(actual.len(), native.len());
        assert_eq!(
            hash(&actual),
            comparison.fontations_hash,
            "Fontations golden: {}",
            original.name
        );
        let mut different = 0;
        let mut maximum = 0;
        let mut total = 0;
        for (actual, native) in actual.iter().zip(&native) {
            different += usize::from(actual != native);
            for (a, b) in [
                (actual[0] & 255, native[0] & 255),
                ((actual[0] >> 8) & 255, (native[0] >> 8) & 255),
                ((actual[0] >> 16) & 255, (native[0] >> 16) & 255),
                (actual[1], native[1]),
            ] {
                let delta = a.abs_diff(b);
                maximum = maximum.max(delta);
                total += delta;
            }
        }
        assert_eq!(
            (different, maximum, total),
            (
                comparison.different_pixels,
                comparison.max_channel_delta,
                comparison.absolute_channel_error
            ),
            "native pixel difference: {}",
            original.name
        );
    }
}

#[test]
fn collection_face_index_is_used_by_fontations_metrics_and_drawing() {
    // Two copies of the synthetic sfnt in a TTC, with distinct hmtx advances.
    // Duplicate names deliberately resolve to face 1, testing its offset rather
    // than merely accepting a collection that silently always draws face 0.
    let first = 20usize;
    let second = (first + FONT.len() + 3) & !3;
    let mut collection = vec![0; second + FONT.len()];
    collection[..12].copy_from_slice(b"ttcf\x00\x01\x00\x00\x00\x00\x00\x02");
    collection[12..16].copy_from_slice(&(first as u32).to_be_bytes());
    collection[16..20].copy_from_slice(&(second as u32).to_be_bytes());
    for base in [first, second] {
        collection[base..base + FONT.len()].copy_from_slice(FONT);
        let count = u16::from_be_bytes(FONT[4..6].try_into().unwrap()) as usize;
        for table in 0..count {
            let record = base + 12 + table * 16;
            let local = u32::from_be_bytes(collection[record + 8..record + 12].try_into().unwrap())
                as usize;
            let offset = base + local;
            collection[record + 8..record + 12].copy_from_slice(&(offset as u32).to_be_bytes());
            if base == second && &collection[record..record + 4] == b"hmtx" {
                collection[offset..offset + 2].copy_from_slice(&900u16.to_be_bytes());
            }
        }
    }
    let mut book = FontBook::default();
    assert_eq!(book.add(collection).unwrap(), 2);
    let glyph = book.rasterize("Kirikiri Synthetic", 'A', 20.0).unwrap();
    assert_eq!(glyph.advance, 18.0);
    assert_eq!(
        book.text_width("Kirikiri Synthetic", &[65], 20, false)
            .unwrap(),
        18
    );
    assert!(glyph.coverage.iter().any(|v| *v != 0));
}
