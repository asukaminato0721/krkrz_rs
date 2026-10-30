use krkrz_runtime::Session;
use krkrz_tjs::Value;

fn playback_session() -> (Session, tempfile::TempDir, tempfile::TempDir) {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("motion.psb"),
        include_bytes!("fixtures/motion.psb"),
    )
    .unwrap();
    std::fs::write(
        project.path().join("startup.tjs"),
        r#"
Plugins.link('motionplayer.dll');
var w=new Window(),root=new Layer(w,null),d=new Layer(w,root);
d.setImageSize(16,8);d.setSizeToImageSize();d.type=ltAlpha;
var manager=new Motion.ResourceManager(w,20971520),document=manager.load('motion.psb');
var player=new Motion.Player(manager);player.chara='hero';
"#,
    )
    .unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), None, 1_000_000).unwrap();
    session.startup().unwrap();
    (session, project, saves)
}

fn script(session: &mut Session, source: &str) {
    let quoted = serde_json::to_string(source).unwrap();
    session
        .evaluate(&format!("Scripts.exec({quoted})"))
        .unwrap();
}

#[test]
fn psb_frames_reach_layer_pixels_and_match_native_timing() {
    let (mut session, _project, _saves) = playback_session();
    script(
        &mut session,
        "player.play('idle',0);player.progress(0);player.clear(d,0);player.draw(d);",
    );
    assert_eq!(session.evaluate("[d.getMaskPixel(0,0),d.getMainPixel(0,0),d.getMainPixel(1,0),d.getMainPixel(0,1)].join(',')").unwrap(), Value::string("255,16711680,65280,255"));
    script(
        &mut session,
        "player.progress(1000/60);player.clear(d,0);player.draw(d);",
    );
    assert_eq!(session.evaluate("[player.frameTickCount,d.getMaskPixel(0,0),d.getMainPixel(2,0),d.getMainPixel(3,0)].join(',')").unwrap(), Value::string("1,0,16711680,65280"));
    assert_eq!(
        session
            .evaluate("player.resourceManager===manager")
            .unwrap(),
        Value::Integer(1)
    );
    assert_eq!(
        session.evaluate("document.id").unwrap(),
        Value::string("motion")
    );
    script(
        &mut session,
        "player.progress(100);player.clear(d,0);player.draw(d);",
    );
    assert_eq!(
        session
            .evaluate("player.playing || d.getMaskPixel(2,0)")
            .unwrap(),
        Value::Integer(0)
    );
}

#[test]
fn stop_seek_speed_and_unloaded_resources_preserve_live_player_state() {
    let (mut session, _project, _saves) = playback_session();
    script(
        &mut session,
        "player.play('idle',Motion.PlayFlagForce);player.progress(100);player.speed=2;player.progress(1000/60);player.stop();player.progress(1000);",
    );
    assert_eq!(
        session.evaluate("player.frameTickCount").unwrap(),
        Value::Real(2.0)
    );
    script(
        &mut session,
        "player.tickCount=0;manager.unloadAll();manager.clearCache();player.draw(d);",
    );
    assert_eq!(
        session.evaluate("d.getMainPixel(0,0)").unwrap(),
        Value::Integer(0xff0000)
    );
    session.collect_garbage(&[]).unwrap();
    script(&mut session, "player.draw(d);invalidate player;");
    let error = session.evaluate("document.id").unwrap_err();
    assert!(format!("{error:#}").contains("invalidated"), "{error:#}");
}

#[test]
fn player_transform_and_destination_clip_control_the_rendered_pixels() {
    let (mut session, _project, _saves) = playback_session();
    script(
        &mut session,
        "player.play('idle',0);player.setCoord(2,2);player.setZoom(2,2);d.setClip(3,2,2,2);player.draw(d);",
    );
    assert_eq!(session.evaluate("[d.getMaskPixel(2,2),d.getMainPixel(3,2),d.getMainPixel(4,2),d.getMaskPixel(5,2)].join(',')").unwrap(), Value::string("0,16711680,65280,0"));
}

#[test]
fn rejected_loads_and_budget_failures_keep_the_previous_pixels_and_model() {
    let (mut session, project, _saves) = playback_session();
    std::fs::write(project.path().join("bad.psb"), b"PSB\0").unwrap();
    assert!(session.evaluate("manager.load('bad.psb')").is_err());
    script(&mut session, "player.play('idle',0);player.draw(d);");
    let pixel = session.evaluate("d.getMainPixel(0,0)").unwrap();
    script(&mut session, "player.tickCount=1000/60;");
    session.budget = 75;
    let error = session.evaluate("player.draw(d)").unwrap_err();
    assert!(
        error.downcast_ref::<krkrz_tjs::VmAbort>().is_some(),
        "{error:#}"
    );
    assert!(
        format!("{error:#}").contains("Motion.Player.draw execution budget exceeded"),
        "{error:#}"
    );
    session.budget = 10000;
    assert_eq!(session.evaluate("d.getMainPixel(0,0)").unwrap(), pixel);
    assert_eq!(
        session.evaluate("d.getMaskPixel(2,0)").unwrap(),
        Value::Integer(0)
    );
}

#[test]
fn rotation_and_discrete_flip_flags_keep_sprite_orientation() {
    let (mut session, _project, _saves) = playback_session();
    script(
        &mut session,
        "player.play('rotate',0);player.progress(0);player.draw(d);",
    );
    assert_eq!(session.evaluate("[d.getMainPixel(3,4),d.getMaskPixel(3,4),d.getMainPixel(3,5),d.getMainPixel(2,4)].join(',')").unwrap(), Value::string("16711680,255,65280,255"));
    script(
        &mut session,
        "player.play('flip',1);player.progress(0);player.progress(1000/60);player.clear(d,0);player.draw(d);",
    );
    assert_eq!(session.evaluate("[d.getMainPixel(0,0),d.getMainPixel(1,0),d.getMaskPixel(0,0),d.getMaskPixel(1,0)].join(',')").unwrap(), Value::string("65280,16711680,255,255"));
}

#[test]
fn loop_step_and_cubic_timelines_select_the_expected_frames() {
    let (mut session, _project, _saves) = playback_session();
    script(
        &mut session,
        "player.play('loop',0);player.progress(0);player.progress(70);",
    );
    assert!(
        (session
            .evaluate("player.tickCount")
            .unwrap()
            .real()
            .unwrap()
            - 20.0)
            .abs()
            < 1e-8
    );
    assert_eq!(
        session.evaluate("player.playing").unwrap(),
        Value::Integer(1)
    );
    script(
        &mut session,
        "player.play('hold',1);player.progress(0);player.progress(1000/60);player.clear(d,0);player.draw(d);",
    );
    assert_eq!(
        session
            .evaluate("[d.getMaskPixel(0,0),d.getMaskPixel(2,0)].join(',')")
            .unwrap(),
        Value::string("255,0")
    );
    script(
        &mut session,
        "player.play('curve',1);player.progress(0);player.progress(1000/120);player.clear(d,0);player.draw(d);",
    );
    assert_eq!(
        session
            .evaluate("[d.getMaskPixel(2,0),d.getMainPixel(3,0),d.getMainPixel(4,0)].join(',')")
            .unwrap(),
        Value::string("0,16711680,65280")
    );
    assert_eq!(
        session
            .evaluate("player.getLayerNames('').join(',')")
            .unwrap(),
        Value::string("sprite")
    );
}

#[test]
fn rl_images_and_psb_name_tables_work_in_versions_two_three_and_four() {
    for bytes in [
        include_bytes!("fixtures/motion.psb").as_slice(),
        include_bytes!("fixtures/motion_v3.psb"),
        include_bytes!("fixtures/motion_v4.psb"),
    ] {
        let (mut session, project, _saves) = playback_session();
        std::fs::write(project.path().join("version.psb"), bytes).unwrap();
        script(
            &mut session,
            "var doc=manager.load('version.psb');player.play('compressed',1);player.progress(0);player.draw(d);",
        );
        assert_eq!(
            session.evaluate("doc.metadata['日本語']").unwrap(),
            Value::string("synthetic")
        );
        assert_eq!(session.evaluate("[d.getMainPixel(0,0),d.getMainPixel(1,0),d.getMainPixel(0,1),d.getMainPixel(1,1)].join(',')").unwrap(), Value::string("16711680,16711680,16711680,16777215"));
    }
}

#[test]
fn motion_plugin_registers_nested_bases_and_preserves_them_on_relink() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("startup.tjs"),
        r#"
Plugins.link('motionplayer.dll');
var motion = Motion, player = Motion.Player, manager = Motion.ResourceManager;
class MotionController extends Motion.Player {}
Plugins.link('motionplayer.dll');
Plugins.link('plugin/MOTIONPLAYER.DLL');
return [Motion === motion, Motion.Player === player,
        Motion.ResourceManager === manager, MotionController instanceof 'Class',
        typeof global.Player, typeof global.ResourceManager,
        Plugins.getList().join(',')].join('|');
"#,
    )
    .unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), None, 10000).unwrap();
    assert_eq!(
        session.startup().unwrap(),
        Value::string("1|1|1|1|undefined|undefined|motionplayer.dll,plugin/MOTIONPLAYER.DLL")
    );
    let error = session.evaluate("new Motion.Player()").unwrap_err();
    assert!(
        format!("{error:#}").contains("missing Motion argument"),
        "{error:#}"
    );
}

#[test]
fn empty_motion_cache_retains_its_owner_until_invalidation() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("startup.tjs"),
        r#"
Plugins.link('motionplayer.dll');
var finalized=0;
class Owner {function finalize(){global.finalized++;}}
var owner=new Owner(), manager=new Motion.ResourceManager(owner,20971520);
owner=null;
manager.clearCache();manager.unloadAll();
"#,
    )
    .unwrap();
    let mut session = Session::open(project.path(), Some(saves.path()), None, 10000).unwrap();
    session.startup().unwrap();
    session.collect_garbage(&[]).unwrap();
    assert_eq!(session.evaluate("finalized").unwrap(), Value::Integer(0));
    let error = session.evaluate("manager.load('test.psb')").unwrap_err();
    assert!(format!("{error:#}").contains("storage not found"));
    session.evaluate("invalidate manager").unwrap();
    session.collect_garbage(&[]).unwrap();
    assert_eq!(session.evaluate("finalized").unwrap(), Value::Integer(1));
}
