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

fn color_image(frame: &ModalFrame) -> egui::ColorImage {
    // Window surfaces retain RGB where the primary Layer does not use alpha.
    // Present those colors as opaque pixels, as presenter.wgsl does.
    egui::ColorImage::new(
        [frame.width as usize, frame.height as usize],
        frame
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .map(|[r, g, b, _]| egui::Color32::from_rgb(*r, *g, *b))
            .collect(),
    )
}

/// Script dialog dimensions are logical points. The desktop scale applies to
/// the window, its bitmap and pointer input together.
struct DialogSize([i32; 2]);
impl DialogSize {
    fn points(&self) -> egui::Vec2 {
        egui::vec2(self.0[0] as f32, self.0[1] as f32)
    }

    fn apply(&self, ctx: &egui::Context) {
        let size = self.points();
        // Keep nonresizable Wayland constraints consistent with the content.
        for command in [
            egui::ViewportCommand::MinInnerSize(size),
            egui::ViewportCommand::MaxInnerSize(size),
            egui::ViewportCommand::InnerSize(size),
        ] {
            ctx.send_viewport_cmd(command);
        }
    }
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
                pixels_per_point: None,
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
    pixels_per_point: Option<f32>,
}
impl App {
    fn event(&mut self, ctx: &egui::Context, event: ModalEvent) -> Result<()> {
        send(&mut std::io::stdout(), &event)?;
        if let Some(frame) = receive(&mut self.input)? {
            validate(&frame)?;
            self.frame = frame;
            self.texture = None;
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.frame.caption.clone()));
            DialogSize(self.frame.client).apply(ctx);
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
        let pixels_per_point = ctx.pixels_per_point();
        let size = DialogSize(self.frame.client);
        let size_error = (ui.max_rect().size() - size.points()).abs();
        // The initial Wayland configure can arrive after the first resize.
        // Retry until the client area matches, allowing logical-pixel rounding.
        if self.pixels_per_point != Some(pixels_per_point) || size_error.max_elem() > 0.5 {
            size.apply(&ctx);
            if self.pixels_per_point.is_none() {
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                    self.frame.position[0] as f32 / pixels_per_point,
                    self.frame.position[1] as f32 / pixels_per_point,
                )));
            }
            self.pixels_per_point = Some(pixels_per_point);
            ctx.request_repaint();
        }
        let texture = self.texture.get_or_insert_with(|| {
            ctx.load_texture(
                "modal-layer-tree",
                color_image(&self.frame),
                egui::TextureOptions::NEAREST,
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
                        x: (pos.x - area.min.x).floor() as i32,
                        y: (pos.y - area.min.y).floor() as i32,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modal_retries_stale_window_size_and_updates_fixed_size_constraints() {
        let ctx = egui::Context::default();
        let mut host = eframe::Frame::_new_kittest();
        let mut app = App {
            frame: ModalFrame {
                caption: String::new(),
                position: [0, 0],
                client: [200, 84],
                rect: [0, 0, 200, 84],
                width: 200,
                height: 84,
                rgba: vec![255; 200 * 84 * 4],
            },
            input: BufReader::new(std::io::stdin()),
            texture: None,
            finished: false,
            pixels_per_point: None,
        };
        for (actual, resizing) in [
            (egui::vec2(400.0, 168.0), true),
            (egui::vec2(400.0, 168.0), true),
            (egui::vec2(200.0, 84.0), false),
        ] {
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, actual)),
                ..Default::default()
            };
            input
                .viewports
                .get_mut(&egui::ViewportId::ROOT)
                .unwrap()
                .native_pixels_per_point = Some(2.0);
            let mut output = ctx.run_ui(input, |ui| eframe::App::ui(&mut app, ui, &mut host));
            let commands = &output.viewport_output[&egui::ViewportId::ROOT].commands;
            let target = egui::vec2(200.0, 84.0);
            for command in [
                egui::ViewportCommand::MinInnerSize(target),
                egui::ViewportCommand::MaxInnerSize(target),
                egui::ViewportCommand::InnerSize(target),
            ] {
                assert_eq!(commands.contains(&command), resizing, "{command:?}");
            }
            output.textures_delta.clear();
        }
    }

    #[test]
    fn modal_pixels_preserve_rgb_when_window_alpha_is_unused() {
        let frame = ModalFrame {
            caption: String::new(),
            position: [0, 0],
            client: [3, 1],
            rect: [0, 0, 3, 1],
            width: 3,
            height: 1,
            rgba: vec![240, 240, 240, 0, 120, 100, 80, 128, 40, 50, 60, 255],
        };
        validate(&frame).unwrap();
        let image = color_image(&frame);
        assert_eq!(image.size, [3, 1]);
        assert_eq!(
            image.pixels,
            [
                egui::Color32::from_rgb(240, 240, 240),
                egui::Color32::from_rgb(120, 100, 80),
                egui::Color32::from_rgb(40, 50, 60),
            ]
        );
    }
}
