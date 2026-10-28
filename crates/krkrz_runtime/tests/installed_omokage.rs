//! The installed game remains external. This check uses a new save directory.
use anyhow::{Context, Result, ensure};
use krkrz_assets::storage::Storage;
use krkrz_core::Limits;
use krkrz_runtime::{
    InputEvent, Session,
    window_modal::{ModalEvent, ModalFrame, ModalWindow},
};
use std::{cell::RefCell, collections::VecDeque, path::Path, rc::Rc};

struct DialogAnswers {
    answers: Rc<RefCell<VecDeque<bool>>>,
    frames: Rc<RefCell<Vec<ModalFrame>>>,
}
impl ModalWindow for DialogAnswers {
    fn next_event(&mut self, frame: &ModalFrame) -> Result<ModalEvent> {
        self.frames.borrow_mut().push(frame.clone());
        let yes = self
            .answers
            .borrow_mut()
            .pop_front()
            .context("dialog requested extra input")?;
        Ok(ModalEvent::Click {
            x: (frame.width * if yes { 651 } else { 1349 } / 2000) as i32,
            y: (frame.height * 66 / 100) as i32,
        })
    }
}

fn advance(session: &mut Session, end: u64) -> Result<()> {
    while session.services.time_ms < end {
        let next = (session.services.time_ms + 64).min(end);
        if session.vm.should_collect_garbage() {
            session.collect_garbage(&[])?;
        }
        session.tick(next)?;
    }
    Ok(())
}

fn click(session: &mut Session, at: u64, x: i32, y: i32) -> Result<()> {
    advance(session, at)?;
    let window = session.evaluate("Window.mainWindow")?;
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
#[ignore = "requires KRKRZ_OMOKAGE_PROJECT_DIR and the installed Omokage game"]
fn title_opening_and_exit_confirmation() -> Result<()> {
    let project =
        std::env::var_os("KRKRZ_OMOKAGE_PROJECT_DIR").context("set KRKRZ_OMOKAGE_PROJECT_DIR")?;
    let mut storage = Storage::open(Path::new(&project), None, Limits::default())?;
    ensure!(
        storage.detect_cipher()? == Some("applique"),
        "unexpected archive cipher"
    );
    let saves = tempfile::tempdir()?;
    let mut session = Session::from_storage(storage, Some(saves.path()), None, 100_000_000_000)?;
    session.startup()?;
    session.tick(0)?;
    for at in [1500, 4000, 8000] {
        click(&mut session, at, 640, 360)?;
    }
    advance(&mut session, 12000)?;
    ensure!(
        session.evaluate("kag.conductor.curStorage")?.text() == "title.ks",
        "title menu did not open"
    );
    click(&mut session, 13000, 230, 400)?;
    for (at, label) in [(17000, "*p1"), (20000, "*p2"), (23000, "*p3")] {
        advance(&mut session, at)?;
        ensure!(
            session.evaluate("kag.currentLabel")?.text() == label,
            "opening dialogue did not reach {label}"
        );
        click(&mut session, at + 500, 800, 650)?;
    }
    let answers = Rc::new(RefCell::new(VecDeque::from([false, true])));
    let frames = Rc::new(RefCell::new(Vec::new()));
    let input = answers.clone();
    let output = frames.clone();
    session.services.set_modal_window_handler(move || {
        Ok(Box::new(DialogAnswers {
            answers: input.clone(),
            frames: output.clone(),
        }))
    });
    session.evaluate("kag.onCloseQuery()")?;
    ensure!(
        session.evaluate("isvalid kag")?.truth()?,
        "cancel closed the game"
    );
    session.evaluate("kag.onCloseQuery()")?;
    ensure!(
        !session.evaluate("isvalid kag")?.truth()?,
        "confirmation did not close the game"
    );
    ensure!(
        answers.borrow().is_empty() && frames.borrow().len() == 2,
        "exit confirmation did not display both dialogs"
    );
    Ok(())
}
