use krkrz_runtime::Session;
use krkrz_tjs::Value;
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    name: String,
    source: String,
    expected: Value,
}

fn assert_resampled_pixels(name: &str, actual: &Value, expected: &Value) {
    // Compare channels, not packed RGB integers: a one-level red difference is
    // 65536 in packed RGB. Keep the original-engine fixtures as the reference.
    // Limits are exclusive (abs_diff < limit), measured with tiny-skia 0.12
    // and fast_image_resize 6.1. Boundary coverage differences need larger
    // limits than interpolation rounding, and are restricted in pixel count.
    let (limits, max_changed, max_large_errors): ([u8; 4], usize, usize) = match name {
        "shrink_integer" | "shrink_self" => ([2; 4], 6, 0),
        "shrink_fraction" => ([243, 197, 174, 46], 16, 16),
        "shrink_clip" => ([144, 119, 107, 10], 9, 9),
        "affine_identity" | "affine_hold_clip" | "affine_outside" | "stretch_nearest" => {
            ([1; 4], 0, 0)
        }
        "affine_rotate_nearest" => ([222, 210, 198, 63], 1, 1),
        "affine_rotate_linear" => ([226, 216, 205, 55], 29, 1),
        "affine_scale_linear" => ([3; 4], 38, 0),
        "affine_no_clip" => ([3; 4], 42, 0),
        "affine_reflect" => ([3; 4], 46, 0),
        "stretch_fast_linear" => ([2, 3, 3, 2], 39, 0),
        "stretch_linear" => ([14, 13, 13, 16], 56, 56),
        "stretch_cubic" | "stretch_cubic_noclip" => ([28, 26, 24, 32], 56, 56),
        "stretch_cubic_coeff" => ([3; 4], 53, 0),
        "stretch_shrink_linear" => ([28, 24, 22, 30], 6, 6),
        "stretch_shrink_cubic" => ([33, 30, 28, 40], 6, 6),
        "stretch_reverse" => ([2; 4], 28, 0),
        "stretch_hold" => ([32, 20, 12, 65], 8, 8),
        _ => panic!("missing pixel tolerances for {name}"),
    };
    let pixels = |value: &Value| {
        assert!(
            matches!(value, Value::String(_)),
            "{name}: expected pixel string"
        );
        value
            .text()
            .split('|')
            .map(|pixel| {
                let (rgb, alpha) = pixel.split_once(':').expect("RGB:alpha pixel");
                let rgb: u32 = rgb.parse().unwrap();
                assert!(rgb <= 0xffffff, "{name}: RGB out of range");
                [
                    (rgb >> 16) as u8,
                    (rgb >> 8) as u8,
                    rgb as u8,
                    alpha.parse::<u8>().unwrap(),
                ]
            })
            .collect::<Vec<_>>()
    };
    let actual = pixels(actual);
    let expected = pixels(expected);
    assert_eq!(actual.len(), expected.len(), "{name}: pixel count");
    let mut changed = 0;
    let mut large_errors = 0;
    for (index, (actual, expected)) in actual.iter().zip(&expected).enumerate() {
        let mut largest = 0;
        for channel in 0..4 {
            let delta = actual[channel].abs_diff(expected[channel]);
            assert!(
                delta < limits[channel],
                "{name}: pixel {index}, channel {}: actual {}, expected {}, abs_diff {delta} >= {}",
                char::from(b"RGBA"[channel]),
                actual[channel],
                expected[channel],
                limits[channel]
            );
            largest = largest.max(delta);
        }
        changed += usize::from(largest > 0);
        large_errors += usize::from(largest > 2);
    }
    assert!(
        changed <= max_changed,
        "{name}: {changed} differing pixels, limit {max_changed}"
    );
    assert!(
        large_errors <= max_large_errors,
        "{name}: {large_errors} pixels differ by more than 2, limit {max_large_errors}"
    );
}

fn session(source: &str, budget: u64) -> (tempfile::TempDir, tempfile::TempDir, Session) {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("case.tjs"), source).unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), None, budget).unwrap();
    session.execute_storage("case.tjs").unwrap();
    (project, saves, session)
}
#[test]
fn system_colors_resolve_for_pixels_fills_and_text() {
    let (_project, _saves, mut session) = session(
        "var w=new Window(),l=new Layer(w,null);l.setSize(40,40);l.face=dfAlpha;l.fillRect(0,0,40,40,0x80445566);l.face=dfOpaque;l.holdAlpha=true;l.fillRect(0,0,1,1,clBtnFace);l.setMainPixel(1,0,clHighlight);l.colorRect(2,0,1,1,clWindow);l.setMainPixel(3,0,0x800000ff);l.setMainPixel(4,0,0x01010203);",
        1_000_000,
    );
    assert_eq!(
        session.evaluate("[l.getMainPixel(0,0),l.getMaskPixel(0,0),l.getMainPixel(1,0),l.getMainPixel(2,0),l.getMainPixel(3,0),l.getMainPixel(4,0)].join(',')").unwrap(),
        Value::string("15790320,128,30935,16777215,0,197121")
    );
    session
        .services
        .fonts
        .add(include_bytes!("fixtures/synthetic.ttf").to_vec())
        .unwrap();
    session.evaluate("Scripts.exec('l.face=dfAlpha;l.holdAlpha=false;l.font.face=\"Kirikiri Synthetic\";l.font.height=20;l.fillRect(0,0,40,40,0);l.drawText(2,2,\"A\",clBtnText,255,true,128,clHighlight,0,2,2);')").unwrap();
    let window = session.evaluate("w").unwrap();
    let themed = session.capture_window(&window).unwrap();
    assert!(
        themed
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .any(|p| p == &[0, 0, 0, 255])
    );
    session.evaluate("Scripts.exec('l.fillRect(0,0,40,40,0);l.drawText(2,2,\"A\",0x000000,255,true,128,0x0078d7,0,2,2);')").unwrap();
    assert_eq!(themed.rgba, session.capture_window(&window).unwrap().rgba);
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
fn node_visibility_tracks_ancestors_and_ignores_opacity_and_enabled_state() {
    let (_, _, mut session) = session(
        "var w=new Window(),p=new Layer(w,null),a=new Layer(w,p),b=new Layer(w,a);b.visible=true;",
        100_000,
    );
    assert_eq!(
        session.evaluate("b.nodeVisible").unwrap(),
        Value::Integer(0)
    );
    session.evaluate("a.visible=true").unwrap();
    assert_eq!(
        session.evaluate("b.nodeVisible").unwrap(),
        Value::Integer(1)
    );
    session
        .evaluate("(a.opacity=0,a.enabled=false,w.visible=false)")
        .unwrap();
    assert_eq!(
        session.evaluate("b.nodeVisible").unwrap(),
        Value::Integer(1)
    );
    session.evaluate("a.visible=false").unwrap();
    assert_eq!(
        session.evaluate("b.nodeVisible").unwrap(),
        Value::Integer(0)
    );
    session.evaluate("invalidate a").unwrap();
    assert_eq!(
        session.evaluate("b.nodeVisible").unwrap(),
        Value::Integer(1)
    );
    assert!(session.evaluate("b.nodeVisible=false").is_err());
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

#[test]
fn original_piled_copy_corpus() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/layer_piled.json")).unwrap();
    for case in cases {
        let (project, _saves, mut session) = session("", 1_000_000);
        std::fs::write(project.path().join("test.tjs"), &case.source).unwrap();
        assert_eq!(
            session
                .execute_storage("test.tjs")
                .unwrap_or_else(|e| panic!("{}: {e:#}", case.name)),
            case.expected,
            "{}",
            case.name
        );
    }
}

#[test]
fn original_transition_corpus() {
    #[derive(Deserialize)]
    struct TransitionCase {
        #[serde(flatten)]
        case: Case,
        advance_ms: Option<u64>,
        result: Option<String>,
    }
    let cases: Vec<TransitionCase> =
        serde_json::from_str(include_str!("fixtures/layer_transition.json")).unwrap();
    for TransitionCase {
        case,
        advance_ms,
        result,
    } in cases
    {
        let (project, _saves, mut session) = session("", 1_000_000);
        std::fs::write(project.path().join("test.tjs"), &case.source).unwrap();
        let mut value = session
            .execute_storage("test.tjs")
            .unwrap_or_else(|e| panic!("{}: {e:#}", case.name));
        if let Some(end) = advance_ms {
            for tick in (0..end).step_by(16).chain([end]) {
                session.tick(tick).unwrap();
            }
            value = session.evaluate(&result.unwrap()).unwrap();
        }
        assert_eq!(value, case.expected, "{}", case.name);
    }
}

#[test]
fn crossfade_capture_and_budget_share_native_tree() {
    let (_project, _saves, mut session) = session(
        "var w=new Window(),p=new Layer(w,null),a=new Layer(w,p),b=new Layer(w,p),clock=0; p.setSize(2,1);p.setImageSize(2,1);a.setSize(2,1);a.setImageSize(2,1);b.setSize(2,1);b.setImageSize(2,1);a.type=ltOpaque;b.type=ltOpaque;a.fillRect(0,0,2,1,0xff000000);b.fillRect(0,0,2,1,0xffffffff);a.visible=true;a.beginTransition('crossfade',true,b,%[time:100,selfupdate:true,callback:function(){return global.clock;}]);",
        100_000,
    );
    let window = session.evaluate("w").unwrap();
    assert_eq!(
        session.capture_window(&window).unwrap().rgba,
        [0, 0, 0, 255].repeat(2)
    );
    session.evaluate("clock=50").unwrap();
    assert_eq!(
        session.capture_window(&window).unwrap().rgba,
        [126, 126, 126, 255].repeat(2)
    );
    session.evaluate("clock=100").unwrap();
    assert_eq!(
        session.capture_window(&window).unwrap().rgba,
        [255, 255, 255, 255].repeat(2)
    );
    assert_eq!(
        session.evaluate("b.visible && !a.visible").unwrap(),
        Value::Integer(1)
    );
    session.budget = 0;
    assert!(session.capture_window(&window).is_err());
}

#[test]
fn original_shrink_copy_corpus() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/layer_shrink.json")).unwrap();
    for case in cases {
        let (_project, _saves, mut session) = session("", 1_000_000);
        let value = session
            .vm
            .execute(
                &krkrz_tjs::compile(&case.name, &case.source).unwrap(),
                &mut session.services,
                &mut session.budget,
            )
            .unwrap_or_else(|e| panic!("{}: {e:#}", case.name));
        assert_resampled_pixels(&case.name, &value, &case.expected);
    }
}

#[test]
fn original_bmp_thumbnail_bytes_and_storage_isolation() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/layer_save.json")).unwrap();
    let (project, saves, mut session) = session(&cases[0].source, 1_000_000);
    for (name, golden) in [
        (
            "bmp8.bmp",
            include_bytes!("fixtures/bmp8-native.bmp").as_slice(),
        ),
        (
            "bmp24.bmp",
            include_bytes!("fixtures/bmp24-native.bmp").as_slice(),
        ),
        (
            "bmp32.bmp",
            include_bytes!("fixtures/bmp32-native.bmp").as_slice(),
        ),
    ] {
        assert_eq!(
            std::fs::read(saves.path().join(name)).unwrap(),
            golden,
            "{name}"
        );
        assert!(!project.path().join(name).exists());
    }
    // The original save script appends serialized TJS after the BMP thumbnail.
    session
        .evaluate("(Dictionary.saveStruct incontextof %['branch'=>1])(global.System.exePath+'bmp8.bmp','o1118')")
        .unwrap();
    let bytes = std::fs::read(saves.path().join("bmp8.bmp")).unwrap();
    assert_eq!(&bytes[..1118], include_bytes!("fixtures/bmp8-native.bmp"));
    assert!(bytes.len() > 1118);
    assert!(
        session
            .evaluate("s.saveLayerImage('../escape.bmp')")
            .is_err()
    );
}

#[test]
fn original_universal_transition_pixels() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/layer_universal.json")).unwrap();
    for case in cases {
        let (project, saves, _session) = session("", 1_000_000);
        std::fs::write(
            project.path().join("transition-rule.png"),
            include_bytes!("fixtures/transition-rule.png"),
        )
        .unwrap();
        // Open after creating the asset so the storage directory catalog sees it.
        let mut session =
            Session::open(project.path(), Some(saves.path()), None, 1_000_000).unwrap();
        let value = session
            .vm
            .execute(
                &krkrz_tjs::compile(&case.name, &case.source).unwrap(),
                &mut session.services,
                &mut session.budget,
            )
            .unwrap_or_else(|e| panic!("{}: {e:#}", case.name));
        assert_eq!(value, case.expected, "{}", case.name);
    }
}

#[test]
fn shrink_errors_and_budget_do_not_modify_pixels() {
    let (_project, _saves, mut session) = session(
        "Plugins.link('PackinOne.dll');var w=new Window(),d=new Layer(w,null),s=new Layer(w,d);d.setImageSize(2,2);s.setImageSize(4,4);d.fillRect(0,0,2,2,0xff123456);",
        100_000,
    );
    for code in [
        "d.shrinkCopy(0,0,2,2,s,0,0,0,4)",
        "d.shrinkCopy(0,0,5,5,s,0,0,4,4)",
        "d.shrinkCopy(0,0,2,2,null,0,0,4,4)",
    ] {
        assert!(session.evaluate(code).is_err(), "{code}");
        assert_eq!(
            session.evaluate("d.getMainPixel(0,0)").unwrap(),
            Value::Integer(0x123456)
        );
    }
    session
        .evaluate("d.shrinkCopy(0,0,2,2,s,100,100,4,4)")
        .unwrap();
    session.budget = 35;
    assert!(session.evaluate("d.shrinkCopy(0,0,2,2,s,0,0,4,4)").is_err());
    session.budget = 1000;
    assert_eq!(
        session.evaluate("d.getMainPixel(0,0)").unwrap(),
        Value::Integer(0x123456)
    );
}

#[test]
fn date_follows_session_wall_clock_origin() {
    let (_project, _saves, mut session) = session("", 10_000);
    session.services.epoch_ms = 1_700_000_000_123;
    assert_eq!(
        session.evaluate("(new Date()).getTime()").unwrap(),
        Value::Integer(1_700_000_000_000)
    );
    session.tick(900).unwrap();
    assert_eq!(
        session.evaluate("(new Date()).getTime()").unwrap(),
        Value::Integer(1_700_000_001_000)
    );
}

#[test]
fn original_affine_copy_pixels() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/layer_affine.json")).unwrap();
    for case in cases {
        let (project, _saves, mut session) = session("", 1_000_000);
        std::fs::write(project.path().join("test.tjs"), &case.source).unwrap();
        let value = session
            .execute_storage("test.tjs")
            .unwrap_or_else(|e| panic!("{}: {e:#}", case.name));
        assert_resampled_pixels(&case.name, &value, &case.expected);
    }
}

#[test]
fn saved_thumbnail_reloads_after_restart_and_prefetch_respects_limits() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/layer_save.json")).unwrap();
    let (project, saves, mut original) = session(&cases[0].source, 1_000_000);
    original.evaluate("(Dictionary.saveStruct incontextof %['branch'=>1])(global.System.exePath+'bmp8.bmp','o1118')").unwrap();
    let expected = original.evaluate("s.getMainPixel(3,2)").unwrap();
    original
        .evaluate("s.loadImages(System.exePath+'bmp8.bmp')")
        .unwrap();
    assert_eq!(
        original.evaluate("s.imageWidth*10+s.imageHeight").unwrap(),
        Value::Integer(75)
    );
    drop(original);
    let mut restarted = Session::open(project.path(), Some(saves.path()), None, 1_000_000).unwrap();
    let setup = "var w=new Window(),s=new Layer(w,null);System.graphicCacheLimit=280;System.touchImages(['missing.png','bmp32.bmp','bmp24.bmp'],140);s.loadImages('bmp32');";
    restarted
        .vm
        .execute(
            &krkrz_tjs::compile("reload.tjs", setup).unwrap(),
            &mut restarted.services,
            &mut restarted.budget,
        )
        .unwrap();
    assert_eq!(restarted.evaluate("s.getMainPixel(3,2)").unwrap(), expected);
    let key32 = saves
        .path()
        .join("bmp32.bmp")
        .to_string_lossy()
        .into_owned();
    let key24 = saves
        .path()
        .join("bmp24.bmp")
        .to_string_lossy()
        .into_owned();
    assert!(restarted.services.image_cache.get(&key32).is_some());
    assert!(restarted.services.image_cache.get(&key24).is_none());
    restarted.evaluate("System.clearGraphicCache()").unwrap();
    assert!(restarted.services.image_cache.get(&key32).is_none());
    restarted
        .evaluate("System.touchImages(['bmp32.bmp'], -280)")
        .unwrap();
    assert!(restarted.services.image_cache.get(&key32).is_none());
    restarted
        .evaluate("System.touchImages(['bmp32.bmp',void,'bmp24.bmp'])")
        .unwrap();
    assert!(restarted.services.image_cache.get(&key32).is_some());
    assert!(restarted.services.image_cache.get(&key24).is_none());
}

#[test]
fn affine_invalid_geometry_and_budget_leave_destination_intact() {
    let (_project, _saves, mut session) = session(
        "var w=new Window(),d=new Layer(w,null),s=new Layer(w,d);d.setImageSize(8,8);s.setImageSize(4,4);d.fillRect(0,0,8,8,0x40123456);",
        100_000,
    );
    for code in [
        "d.affineCopy(s,-1,0,4,4,true,1,0,0,1,0,0)",
        "d.affineCopy(s,0,0,5,4,true,1,0,0,1,0,0)",
        "d.affineCopy(s,0,0,4,4,true,1,0,0,1,1e100,0)",
        "d.affineCopy(s,0,0,4,4,false,0,0,0.0000152587890625,0,0,1)",
    ] {
        assert!(session.evaluate(code).is_err(), "{code}");
        assert_eq!(
            session.evaluate("d.getMainPixel(0,0)").unwrap(),
            Value::Integer(0x123456)
        );
    }
    session.budget = 60;
    assert!(
        session
            .evaluate("d.affineCopy(s,0,0,4,4,true,1,0,0,1,0,0,0,true)")
            .is_err()
    );
    session.budget = 1000;
    assert_eq!(
        session.evaluate("d.getMainPixel(0,0)").unwrap(),
        Value::Integer(0x123456)
    );
}

#[test]
fn original_stretch_copy_pixels() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/layer_stretch.json")).unwrap();
    for case in cases {
        let (project, _saves, mut session) = session("", 1_000_000);
        std::fs::write(project.path().join("test.tjs"), &case.source).unwrap();
        let value = session
            .execute_storage("test.tjs")
            .unwrap_or_else(|e| panic!("{}: {e:#}", case.name));
        assert_resampled_pixels(&case.name, &value, &case.expected);
    }
}
