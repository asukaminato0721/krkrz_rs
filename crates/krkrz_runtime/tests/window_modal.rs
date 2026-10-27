use anyhow::{Context, Result};
use krkrz_runtime::{
    Session,
    window_modal::{ModalEvent, ModalFrame, ModalWindow},
};
use krkrz_tjs::Value;
use std::{cell::RefCell, collections::VecDeque, rc::Rc};

struct Host {
    events: VecDeque<ModalEvent>,
    frames: Rc<RefCell<Vec<ModalFrame>>>,
}
impl ModalWindow for Host {
    fn next_event(&mut self, frame: &ModalFrame) -> Result<ModalEvent> {
        self.frames.borrow_mut().push(frame.clone());
        self.events
            .pop_front()
            .context("no modal test input remains")
    }
}
fn session(
    project: &std::path::Path,
    saves: &std::path::Path,
    events: Vec<ModalEvent>,
) -> (Session, Rc<RefCell<Vec<ModalFrame>>>) {
    let mut session = Session::open(project, Some(saves), None, 1_000_000_000).unwrap();
    let frames = Rc::new(RefCell::new(Vec::new()));
    let output = frames.clone();
    session.services.set_modal_window_handler(move || {
        Ok(Box::new(Host {
            events: events.clone().into(),
            frames: output.clone(),
        }))
    });
    (session, frames)
}

#[test]
fn modal_window_preserves_result_and_pixels_until_explicit_invalidation() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    let (mut session, frames) = session(
        project.path(),
        saves.path(),
        vec![ModalEvent::Click { x: 4, y: 5 }],
    );
    let source = r#"
global.main=new Window();
class Dialog extends Window {
    var result=false;
    function Dialog(){super.Window();caption='Confirmation';innerSunken=false;showScrollBars=false;setInnerSize(20,16);var p=new Layer(this,null);p.setSize(20,16);p.fillRect(0,0,20,16,0xff123456);}
    function onClick(x,y){result=x==4 && y==5;close();}
}
var d=new Dialog();d.showModal();
var result=[d.result,isvalid d,d.visible,isvalid main,Window.mainWindow===main];
invalidate d;result.add(isvalid d);return result.join('|');
"#;
    std::fs::write(project.path().join("case.tjs"), source).unwrap();
    assert_eq!(
        session.execute_storage("case.tjs").unwrap(),
        Value::string("1|1|0|1|1|0")
    );
    assert_eq!(frames.borrow().len(), 1);
    let frames = frames.borrow();
    let frame = &frames[0];
    assert_eq!((frame.width, frame.height), (20, 16));
    assert!(
        frame
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| *pixel == [0x12, 0x34, 0x56, 255])
    );
}

#[test]
fn modal_close_can_be_rejected_and_backend_failure_restores_window_state() {
    let project = tempfile::tempdir().unwrap();
    let saves = tempfile::tempdir().unwrap();
    let (mut session, frames) = session(
        project.path(),
        saves.path(),
        vec![ModalEvent::Close, ModalEvent::Key { code: 27 }],
    );
    std::fs::write(project.path().join("case.tjs"), "global.w=new Window();var p=new Layer(w,null),closeQuery=w.onCloseQuery;w.onCloseQuery=function(){this.queries++;};w.queries=0;w.onKeyDown=function(key,shift){closeQuery(true);};w.showModal();return w.queries;").unwrap();
    assert_eq!(
        session.execute_storage("case.tjs").unwrap(),
        Value::Integer(1)
    );
    assert_eq!(frames.borrow().len(), 2);
    session.services.set_modal_window_handler(|| {
        Ok(Box::new(Host {
            events: VecDeque::new(),
            frames: Rc::default(),
        }))
    });
    assert!(session.evaluate("w.showModal()").is_err());
    assert_eq!(session.evaluate("w.visible").unwrap(), Value::Integer(0));
    assert_eq!(session.evaluate("isvalid w").unwrap(), Value::Integer(1));
}

#[test]
#[ignore = "requires the installed Sweet Girl game and KRKRZ_PROJECT_DIR"]
fn original_yes_no_dialog_returns_both_answers() {
    let project = std::env::var_os("KRKRZ_PROJECT_DIR").unwrap();
    for (event, expected) in [
        (ModalEvent::Key { code: 27 }, 0),
        (ModalEvent::Close, 0),
        (ModalEvent::Click { x: 60, y: 58 }, 1),
    ] {
        let saves = tempfile::tempdir().unwrap();
        let (mut session, frames) =
            session(std::path::Path::new(&project), saves.path(), vec![event]);
        session.execute_storage("startup.tjs").unwrap();
        for time in (0..=15000).step_by(16) {
            session.tick(time).unwrap();
        }
        assert_eq!(
            session.evaluate("askYesNo('終了しますか？')").unwrap(),
            Value::Integer(expected)
        );
        assert!(session.evaluate("isvalid kag").unwrap().truth().unwrap());
        assert_eq!(frames.borrow().len(), 1);
        if let Some(path) = std::env::var_os("KRKRZ_MODAL_FRAME") {
            std::fs::write(path, serde_json::to_vec(&frames.borrow()[0]).unwrap()).unwrap();
        }
    }
}

#[test]
#[ignore = "requires the installed Sweet Girl game and KRKRZ_PROJECT_DIR"]
fn original_exit_link_completes_its_input_callback() {
    use krkrz_runtime::InputEvent;
    let project = std::env::var_os("KRKRZ_PROJECT_DIR").unwrap();
    let saves = tempfile::tempdir().unwrap();
    let (mut session, _) = session(
        std::path::Path::new(&project),
        saves.path(),
        vec![ModalEvent::Click { x: 60, y: 58 }],
    );
    session.execute_storage("startup.tjs").unwrap();
    for time in (0..=15000).step_by(16) {
        session.tick(time).unwrap();
    }
    let window = session.evaluate("kag").unwrap();
    for event in [
        InputEvent::PointerMove {
            x: 970,
            y: 487,
            shift: 0,
        },
        InputEvent::PointerDown {
            x: 970,
            y: 487,
            button: 0,
            shift: 8,
        },
        InputEvent::Click { x: 970, y: 487 },
        InputEvent::PointerUp {
            x: 970,
            y: 487,
            button: 0,
            shift: 0,
        },
    ] {
        if session.evaluate("isvalid kag").unwrap().truth().unwrap() {
            session.input(&window, event).unwrap();
        }
    }
    assert!(!session.evaluate("isvalid kag").unwrap().truth().unwrap());
}

#[test]
#[ignore = "requires the installed Yuri game and KRKRZ_PROJECT_DIR"]
fn original_yuri_dialog_parses_settings_and_returns_both_answers() {
    use krkrz_assets::storage::Storage;
    use krkrz_core::Limits;
    let project = std::env::var_os("KRKRZ_PROJECT_DIR").unwrap();
    let saves = tempfile::tempdir().unwrap();
    let mut storage =
        Storage::open(std::path::Path::new(&project), None, Limits::default()).unwrap();
    storage.detect_cipher().unwrap();
    let mut session =
        Session::from_storage(storage, Some(saves.path()), None, 1_000_000_000).unwrap();
    let frames = Rc::new(RefCell::new(Vec::new()));
    let output = frames.clone();
    let mut inputs = VecDeque::from([
        ModalEvent::Key { code: 27 },
        ModalEvent::Click { x: 220, y: 95 },
        ModalEvent::Click { x: 70, y: 95 },
    ]);
    session.services.set_modal_window_handler(move || {
        Ok(Box::new(Host {
            events: VecDeque::from([inputs.pop_front().context("no dialog test input remains")?]),
            frames: output.clone(),
        }))
    });
    session.startup().unwrap();
    for time in (0..=10000).step_by(16) {
        session.tick(time).unwrap();
    }
    for expected in [0, 0, 1] {
        assert_eq!(
            session.evaluate("askYesNo('終了しますか？')").unwrap(),
            Value::Integer(expected)
        );
        assert_eq!(
            session.evaluate("isvalid tWindow").unwrap(),
            Value::Integer(1)
        );
    }
    let frames = frames.borrow();
    assert_eq!(frames.len(), 3);
    assert!(frames[0].width > 220 && frames[0].height > 95);
    if let Some(path) = std::env::var_os("KRKRZ_MODAL_FRAME") {
        std::fs::write(path, serde_json::to_vec(&frames[0]).unwrap()).unwrap();
    }
}
