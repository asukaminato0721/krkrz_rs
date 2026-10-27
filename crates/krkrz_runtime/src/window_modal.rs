//! Synchronous script windows use the same compositor and input dispatch as the host.
use crate::{InputEvent, Services, layer::object};
use anyhow::{Context, Result, ensure};
use krkrz_tjs::{Value, Vm, unsupported};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModalFrame {
    pub caption: String,
    pub position: [i32; 2],
    pub client: [i32; 2],
    pub rect: [i32; 4],
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ModalEvent {
    Click { x: i32, y: i32 },
    Key { code: u32 },
    Close,
}

/// One modal presentation. Drop closes its host resources.
pub trait ModalWindow {
    fn next_event(&mut self, frame: &ModalFrame) -> Result<ModalEvent>;
}

type Handler = Box<dyn FnMut() -> Result<Box<dyn ModalWindow>>>;
#[derive(Default)]
pub(crate) struct State {
    handler: Option<Handler>,
}

impl Services {
    pub fn set_modal_window_handler(
        &mut self,
        handler: impl FnMut() -> Result<Box<dyn ModalWindow>> + 'static,
    ) {
        self.modal_windows.handler = Some(Box::new(handler));
    }

    pub(crate) fn show_window_modal(
        &mut self,
        vm: &mut Vm,
        context: &Value,
        budget: &mut u64,
    ) -> Result<Value> {
        let id = object(context)?.context("Window.showModal requires a Window")?;
        let window = self
            .windows
            .get(&id)
            .filter(|w| w.constructed)
            .context("Window.showModal requires a constructed Window")?;
        ensure!(!window.modal, "Window is already modal");
        let mut host = self
            .modal_windows
            .handler
            .as_mut()
            .context("Window.showModal requires a modal window handler")?()?;
        let window = self.windows.get_mut(&id).unwrap();
        window.modal = true;
        window.visible = true;
        let result = self.modal_loop(vm, context, host.as_mut(), budget);
        if let Some(window) = self.windows.get_mut(&id) {
            window.modal = false;
            window.visible = false;
            window.input.keys.clear();
            window.input.capture = None;
        }
        result?;
        Ok(Value::Void)
    }

    fn modal_loop(
        &mut self,
        vm: &mut Vm,
        context: &Value,
        host: &mut dyn ModalWindow,
        budget: &mut u64,
    ) -> Result<()> {
        let id = object(context)?.unwrap();
        while self.windows.get(&id).is_some_and(|w| w.modal && w.visible) {
            *budget = budget
                .checked_sub(1)
                .ok_or_else(|| unsupported("modal window execution budget exceeded"))?;
            let root = object(&self.windows[&id].primary_layer)?
                .context("modal window has no primary Layer")?;
            self.layer_before_completion(vm, root, budget)?;
            let image = self.capture_window_prepared(vm, context, budget)?;
            let Some(window) = self.windows.get(&id).filter(|w| w.modal && w.visible) else {
                return Ok(());
            };
            let frame = ModalFrame {
                caption: window.caption.clone(),
                position: [window.left, window.top],
                client: [window.inner_width, window.inner_height],
                rect: window.draw_rect(image.width as i32, image.height as i32),
                width: image.width,
                height: image.height,
                rgba: image.rgba,
            };
            let events = match host.next_event(&frame)? {
                ModalEvent::Click { x, y } => vec![
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
                ],
                ModalEvent::Key { code } => vec![
                    InputEvent::KeyDown {
                        key: code,
                        shift: 0,
                    },
                    InputEvent::KeyUp {
                        key: code,
                        shift: 0,
                    },
                ],
                ModalEvent::Close => {
                    self.host_window_event(vm, id, "onCloseQuery", &[Value::Integer(1)], budget)?;
                    Vec::new()
                }
            };
            for event in events {
                if self.windows.get(&id).is_some_and(|w| w.modal && w.visible) {
                    self.window_input(vm, context, event, budget)?;
                }
            }
        }
        Ok(())
    }
}
