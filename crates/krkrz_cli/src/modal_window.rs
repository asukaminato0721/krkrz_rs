//! Render script modal windows in a child process while the VM waits for input.
use anyhow::{Context, Result, ensure};
use eframe::egui;
use krkrz_runtime::window_modal::{ModalEvent, ModalFrame, ModalWindow};
use std::{
    cell::Cell,
    io::{BufRead, BufReader, Read, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    rc::Rc,
    time::{Duration, Instant},
};

const MAX_MESSAGE: usize = 16 << 20;

fn receive<T: serde::de::DeserializeOwned>(reader: &mut impl BufRead) -> Result<Option<T>> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_MESSAGE as u64 + 1)
        .read_until(b'\n', &mut bytes)?;
    ensure!(
        bytes.len() <= MAX_MESSAGE,
        "modal message exceeds size limit"
    );
    if bytes.is_empty() {
        return Ok(None);
    }
    Ok(Some(serde_json::from_slice(&bytes)?))
}

fn send(writer: &mut impl Write, value: &impl serde::Serialize) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    ensure!(
        bytes.len() <= MAX_MESSAGE,
        "modal message exceeds size limit"
    );
    writer.write_all(&bytes)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

pub struct Process {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    wait: Rc<Cell<Duration>>,
}
impl Process {
    pub fn start(wait: Rc<Cell<Duration>>) -> Result<Box<dyn ModalWindow>> {
        let mut child = Command::new(std::env::current_exe()?)
            .arg("--modal-window")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .context("start modal window process")?;
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        Ok(Box::new(Self {
            child,
            input,
            output,
            wait,
        }))
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl ModalWindow for Process {
    fn next_event(&mut self, frame: &ModalFrame) -> Result<ModalEvent> {
        let start = Instant::now();
        let result = (|| {
            send(&mut self.input, frame)?;
            receive(&mut self.output)?.context("modal window closed without an input event")
        })();
        self.wait.set(self.wait.get() + start.elapsed());
        result
    }
}

fn validate(frame: &ModalFrame) -> Result<()> {
    ensure!(
        (1..=4096).contains(&frame.width) && (1..=4096).contains(&frame.height),
        "invalid modal bitmap size"
    );
    ensure!(
        frame.rgba.len() == frame.width as usize * frame.height as usize * 4,
        "invalid modal bitmap data"
    );
    ensure!(
        frame.client.iter().all(|size| (1..=8192).contains(size)),
        "invalid modal client size"
    );
    Ok(())
}

pub fn run_child() -> Result<()> {
    let mut input = BufReader::new(std::io::stdin());
    let frame: ModalFrame = receive(&mut input)?.context("missing modal window frame")?;
    validate(&frame)?;
    let viewport = egui::ViewportBuilder::default()
        .with_title(&frame.caption)
        .with_position([frame.position[0] as f32, frame.position[1] as f32])
        .with_inner_size([frame.client[0] as f32, frame.client[1] as f32])
        .with_resizable(false);
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "krkrz-modal-window",
        options,
        Box::new(move |_| {
            Ok(Box::new(App {
                frame,
                input,
                texture: None,
                finished: false,
            }))
        }),
    )
    .map_err(|error| anyhow::anyhow!("modal window: {error}"))
}

struct App {
    frame: ModalFrame,
    input: BufReader<std::io::Stdin>,
    texture: Option<egui::TextureHandle>,
    finished: bool,
}
impl App {
    fn event(&mut self, ctx: &egui::Context, event: ModalEvent) -> Result<()> {
        send(&mut std::io::stdout(), &event)?;
        if let Some(frame) = receive(&mut self.input)? {
            validate(&frame)?;
            self.frame = frame;
            self.texture = None;
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.frame.caption.clone()));
        } else {
            self.finished = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        Ok(())
    }
}
impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.finished {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        let texture = self.texture.get_or_insert_with(|| {
            ctx.load_texture(
                "modal-layer-tree",
                egui::ColorImage::from_rgba_unmultiplied(
                    [self.frame.width as usize, self.frame.height as usize],
                    &self.frame.rgba,
                ),
                egui::TextureOptions::LINEAR,
            )
        });
        let mut event = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(egui::Color32::BLACK))
            .show(ui, |ui| {
                let area = ui.max_rect();
                let [x, y, w, h] = self.frame.rect;
                let rect = egui::Rect::from_min_size(
                    area.min + egui::vec2(x as f32, y as f32),
                    egui::vec2(w as f32, h as f32),
                );
                ui.painter().image(
                    texture.id(),
                    rect,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
                let response =
                    ui.interact(area, egui::Id::new("modal-input"), egui::Sense::click());
                if response.clicked()
                    && let Some(pos) = response.interact_pointer_pos()
                {
                    event = Some(ModalEvent::Click {
                        x: (pos.x - area.min.x) as i32,
                        y: (pos.y - area.min.y) as i32,
                    });
                }
            });
        for (key, code) in [
            (egui::Key::Escape, 27),
            (egui::Key::Enter, 13),
            (egui::Key::Tab, 9),
            (egui::Key::Space, 32),
            (egui::Key::ArrowLeft, 37),
            (egui::Key::ArrowUp, 38),
            (egui::Key::ArrowRight, 39),
            (egui::Key::ArrowDown, 40),
        ] {
            if ctx.input(|input| input.key_pressed(key)) {
                event = Some(ModalEvent::Key { code });
            }
        }
        if ctx.input(|input| input.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            event = Some(ModalEvent::Close);
        }
        if let Some(event) = event
            && let Err(error) = self.event(&ctx, event)
        {
            eprintln!("modal window: {error:#}");
            self.finished = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}
