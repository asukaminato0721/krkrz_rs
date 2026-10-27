use anyhow::{Context, Result};
use krkrz_assets::storage::Storage;
use krkrz_core::Limits;
use krkrz_runtime::{
    InputEvent, Session,
    window_modal::{ModalEvent, ModalFrame, ModalWindow},
};
use krkrz_tjs::Value;
use std::path::Path;

struct DeclineFullscreen;
impl ModalWindow for DeclineFullscreen {
    fn next_event(&mut self, _: &ModalFrame) -> Result<ModalEvent> {
        Ok(ModalEvent::Key { code: 27 })
    }
}

fn advance(session: &mut Session, end: u64) -> Result<()> {
    let start = session.services.time_ms;
    for time in (start..=end).step_by(16) {
        if session.vm.should_collect_garbage() {
            session.collect_garbage(&[])?;
        }
        session
            .tick(time)
            .with_context(|| format!("gallery tick {time}"))?;
    }
    session.tick(end)
}

fn click(session: &mut Session, x: i32, y: i32) -> Result<()> {
    let window = session.evaluate("kag")?;
    for event in [
        InputEvent::PointerMove { x, y, shift: 0 },
        InputEvent::PointerDown {
            x,
            y,
            button: 0,
            shift: 8,
        },
        InputEvent::Click { x, y },
        InputEvent::PointerUp {
            x,
            y,
            button: 0,
            shift: 0,
        },
    ] {
        session.input(&window, event)?;
    }
    Ok(())
}

#[test]
#[ignore = "requires the installed Joshi game and KRKRZ_PROJECT_DIR"]
fn original_scene_gallery_expands_live_macro_parameters() -> Result<()> {
    let project = std::env::var_os("KRKRZ_PROJECT_DIR").context("set KRKRZ_PROJECT_DIR")?;
    let saves = tempfile::tempdir()?;
    let mut storage = Storage::open(Path::new(&project), None, Limits::default())?;
    storage.detect_cipher()?;
    let mut session = Session::from_storage(storage, Some(saves.path()), None, 100_000_000_000)?;
    session
        .services
        .set_modal_window_handler(|| Ok(Box::new(DeclineFullscreen)));
    session.startup()?;
    advance(&mut session, 12000)?;
    let x = session
        .evaluate("titlemenu_object.foreConfig.ScenesButton.left+10")?
        .integer()? as i32;
    let y = session
        .evaluate("titlemenu_object.foreConfig.ScenesButton.top+10")?
        .integer()? as i32;
    click(&mut session, x, y)?;
    advance(&mut session, 14000)?;
    assert_eq!(
        session.evaluate("kag.conductor.curStorage")?,
        Value::string("SceneList.ks")
    );
    assert_eq!(
        session.evaluate("f.cgdata.current_page_index")?,
        Value::Integer(0)
    );
    assert_eq!(session.evaluate("tf.linkno")?, Value::Integer(12));
    assert!(
        session
            .evaluate("kag.fore.messages[0].links[0].object.width > 0")?
            .truth()?
    );
    let window = session.evaluate("kag")?;
    let frame = session.capture_window(&window)?;
    assert_eq!((frame.width, frame.height), (1280, 720));
    if let Some(path) = std::env::var_os("KRKRZ_GALLERY_FRAME") {
        frame.write_png(Path::new(&path))?;
    }
    click(&mut session, 1085, 70)?;
    advance(&mut session, 16000)?;
    assert_eq!(
        session.evaluate("f.cgdata.current_page_index")?,
        Value::Integer(4)
    );
    assert_eq!(session.evaluate("tf.linkno")?, Value::Integer(2));
    click(&mut session, 575, 70)?;
    advance(&mut session, 18000)?;
    assert_eq!(
        session.evaluate("f.cgdata.current_page_index")?,
        Value::Integer(0)
    );
    assert_eq!(session.evaluate("tf.linkno")?, Value::Integer(12));
    click(&mut session, 1160, 680)?;
    advance(&mut session, 20000)?;
    assert_eq!(
        session.evaluate("kag.conductor.curStorage")?,
        Value::string("first.ks")
    );
    assert_eq!(session.evaluate("f.gallery")?, Value::Integer(0));
    Ok(())
}
