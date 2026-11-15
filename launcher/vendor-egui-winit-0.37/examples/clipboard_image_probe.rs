//! Opt-in native Wayland clipboard image probe built on the real `egui_winit::State`.
//!
//! This example is meant to run only against a private, disposable headless compositor. It never
//! consults the user's session: the caller supplies `XDG_RUNTIME_DIR`, `WAYLAND_DISPLAY`, and a
//! private `SWAYSOCK`. The companion `clipboard_image_smoke.py` starts that compositor and drives
//! the probe with `wtype`, `wl-copy`, and `wl-paste`.
//!
//! Normal mode maps a real window through softbuffer and pumps egui's frame lifecycle:
//! `State::on_window_event`, `State::take_egui_input`, and
//! `State::handle_platform_output_with_event_loop` each logical frame. egui repaint requests are
//! forwarded through an `EventLoopProxy`, which is what lets asynchronous clipboard completion
//! and image encoding wake the event loop.
//!
//! Controls (while the probe window has keyboard focus):
//! * `i` - emit `OutputCommand::CopyImage` with the deterministic 2x2 seed image
//! * `t` - emit `OutputCommand::CopyText` with the probe text
//! * `m` - emit image then text in one batch (text must win as the last regular command)
//! * `n` - emit text then image in one batch (image must win as the last regular command)
//! * Ctrl+V - run the normal egui-winit paste path (external or self selection)
//! * `q` - quit
//!
//! The probe logs one line per raw `egui::Event::Paste` / `Event::PasteImage`, including length or
//! dimensions and deterministic hashes. It does not consume or write any clipboard outside the
//! private compositor.
//!
//! `--verify-png FILE` decodes an externally copied PNG and prints its exact straight-alpha RGBA,
//! alpha channel, and whether it matches the probe's 2x2 seed. It exits non-zero on mismatch.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "stdout/stderr carry this probe's test protocol and diagnostics"
)]

#[cfg(all(
    target_os = "linux",
    feature = "wayland",
    feature = "clipboard",
    feature = "clipboard-image"
))]
mod probe {
    use core::{fmt::Write as _, num::NonZeroU32, time::Duration};
    use std::{path::Path, sync::Arc, time::Instant};

    use egui::mutex::Mutex;
    use egui_winit::egui::{self, ColorImage, OutputCommand, ViewportId};
    use winit::{
        application::ApplicationHandler,
        dpi::PhysicalSize,
        event::{ElementState, WindowEvent},
        event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
        keyboard::Key,
        platform::wayland::{ActiveEventLoopExtWayland as _, EventLoopBuilderExtWayland as _},
        window::{Window, WindowId},
    };

    const TEXT: &str = "native clipboard text 🦀\nsecond line";
    const SEED_RGBA: [[u8; 4]; 4] = [
        [255, 0, 0, 255], // opaque red
        [0, 255, 0, 128], // half-alpha green
        [0, 0, 255, 64],  // quarter-alpha blue
        [0, 0, 0, 0],     // fully transparent
    ];
    const WINDOW_TITLE: &str = "egui-winit native clipboard image probe";
    const PROBE_BUDGET: Duration = Duration::from_mins(2);

    #[derive(Clone, Copy, Debug)]
    enum UserEvent {
        Wake,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Command {
        CopyImage,
        CopyText,
        ImageThenText,
        TextThenImage,
    }

    impl Command {
        fn label(self) -> &'static str {
            match self {
                Self::CopyImage => "CopyImage",
                Self::CopyText => "CopyText",
                Self::ImageThenText => "ImageThenText",
                Self::TextThenImage => "TextThenImage",
            }
        }
    }

    fn seed_rgba_bytes() -> [u8; 16] {
        [255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 64, 0, 0, 0, 0]
    }

    fn seed_image() -> ColorImage {
        let rgba = seed_rgba_bytes();
        ColorImage::from_rgba_unmultiplied([2, 2], &rgba)
    }

    fn color_rgba(image: &ColorImage) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(image.pixels.len() * 4);
        for pixel in &image.pixels {
            bytes.extend_from_slice(&pixel.to_srgba_unmultiplied());
        }
        bytes
    }

    fn premultiplied_rgba(image: &ColorImage) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(image.pixels.len() * 4);
        for pixel in &image.pixels {
            bytes.extend_from_slice(&pixel.to_array());
        }
        bytes
    }

    fn alpha_bytes(image: &ColorImage) -> Vec<u8> {
        image.pixels.iter().map(|pixel| pixel.a()).collect()
    }

    fn hex(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            let _ = write!(out, "{byte:02x}");
        }
        out
    }

    fn fnv1a(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
    }

    fn seed_matches(image: &ColorImage) -> bool {
        image.size == [2, 2]
            && image.pixels.len() == SEED_RGBA.len()
            && image
                .pixels
                .iter()
                .zip(SEED_RGBA)
                .all(|(pixel, expected)| pixel.to_srgba_unmultiplied() == expected)
    }

    fn log_image(label: &str, frame: u64, index: usize, image: &ColorImage) {
        let rgba = color_rgba(image);
        let premultiplied = premultiplied_rgba(image);
        let alpha = alpha_bytes(image);
        println!(
            "{label} frame={frame} index={index} dims={}x{} pixels={} rgba_hex={} rgba_fnv={:016x} premult_hex={} alpha_hex={} seed_match={}",
            image.size[0],
            image.size[1],
            image.pixels.len(),
            hex(&rgba),
            fnv1a(&rgba),
            hex(&premultiplied),
            hex(&alpha),
            seed_matches(image),
        );
    }

    struct Renderer {
        // Fields drop top-to-bottom: the surface must go before the context that owns its display
        // state. Both handles also own an `Arc<Window>`, so window drop order is irrelevant.
        surface: softbuffer::Surface<Arc<Window>, Arc<Window>>,
        _context: softbuffer::Context<Arc<Window>>,
    }

    impl Renderer {
        fn new(window: &Arc<Window>) -> Result<Self, String> {
            let context = softbuffer::Context::new(Arc::clone(window))
                .map_err(|error| format!("softbuffer context: {error}"))?;
            let surface = softbuffer::Surface::new(&context, Arc::clone(window))
                .map_err(|error| format!("softbuffer surface: {error}"))?;
            Ok(Self {
                surface,
                _context: context,
            })
        }

        fn present(&mut self, frame: u64) -> Result<(u32, u32), String> {
            let size = self.surface.window().inner_size();
            let (Some(width), Some(height)) =
                (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
            else {
                // A first RedrawRequested before the initial configure is harmless: a later
                // Resized/redraw presents once the surface has a real size.
                return Ok((0, 0));
            };
            self.surface
                .resize(width, height)
                .map_err(|error| format!("softbuffer resize: {error}"))?;
            let mut buffer = self
                .surface
                .buffer_mut()
                .map_err(|error| format!("softbuffer buffer: {error}"))?;
            let buffer_width = buffer.width().get();
            let buffer_height = buffer.height().get();

            for (index, pixel) in buffer.iter_mut().enumerate() {
                let x = index as u32 % buffer_width;
                let y = index as u32 / buffer_width;
                let checker = ((x / 32) + (y / 32)) & 1 == 0;
                let (mut red, mut green, mut blue): (u8, u8, u8) =
                    if checker { (24, 28, 36) } else { (16, 18, 24) };

                // A visible status strip proves an actual present, not just CPU buffer writes.
                if y < 8 {
                    green = 96 + (frame % 128) as u8;
                }
                // Paint the four seed colors as large swatches.
                if (80..144).contains(&x) && (80..144).contains(&y) {
                    (red, green, blue) = (255, 0, 0);
                } else if (160..224).contains(&x) && (80..144).contains(&y) {
                    (red, green, blue) = (0, 160, 0);
                } else if (80..144).contains(&x) && (160..224).contains(&y) {
                    (red, green, blue) = (0, 0, 255);
                }

                *pixel = (u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue);
            }

            buffer
                .present()
                .map_err(|error| format!("softbuffer present: {error}"))?;
            Ok((buffer_width, buffer_height))
        }
    }

    struct App {
        egui_ctx: egui::Context,
        state: Option<egui_winit::State>,
        window: Option<Arc<Window>>,
        renderer: Option<Renderer>,
        pending_commands: Vec<Command>,
        pending_origins: Vec<String>,
        modifiers_ctrl: bool,
        frame: u64,
        window_events: u64,
        started: Instant,
        mapped: bool,
        presented_once: bool,
    }

    impl App {
        fn new(egui_ctx: egui::Context) -> Self {
            Self {
                egui_ctx,
                state: None,
                window: None,
                renderer: None,
                pending_commands: Vec::new(),
                pending_origins: Vec::new(),
                modifiers_ctrl: false,
                frame: 0,
                window_events: 0,
                started: Instant::now(),
                mapped: false,
                presented_once: false,
            }
        }

        fn log_raw_input(&self, raw_input: &egui::RawInput) {
            for (index, event) in raw_input.events.iter().enumerate() {
                match event {
                    egui::Event::Paste(text) => {
                        println!(
                            "RAW_PASTE frame={} index={} bytes={} fnv={:016x} hex={}",
                            self.frame,
                            index,
                            text.len(),
                            fnv1a(text.as_bytes()),
                            hex(text.as_bytes()),
                        );
                    }
                    egui::Event::PasteImage(image) => {
                        log_image("RAW_PASTE_IMAGE", self.frame, index, image);
                    }
                    egui::Event::MiddleClickPaste { pos, text } => {
                        println!(
                            "RAW_PRIMARY_PASTE frame={} index={} x={:.2} y={:.2} bytes={} fnv={:016x} hex={}",
                            self.frame,
                            index,
                            pos.x,
                            pos.y,
                            text.len(),
                            fnv1a(text.as_bytes()),
                            hex(text.as_bytes()),
                        );
                    }
                    egui::Event::Copy => {
                        println!("RAW_COPY frame={} index={}", self.frame, index);
                    }
                    egui::Event::Cut => {
                        println!("RAW_CUT frame={} index={}", self.frame, index);
                    }
                    egui::Event::Key {
                        key, pressed: true, ..
                    } => {
                        println!("RAW_KEY frame={} index={} key={key:?}", self.frame, index);
                    }
                    _ => {}
                }
            }
        }

        fn note_key_origin(&mut self, window: &Arc<Window>, origin: String) {
            println!("KEY_TRIGGER {origin}");
            self.pending_origins.push(origin);
            window.request_redraw();
        }

        fn handle_key(&mut self, event_loop: &ActiveEventLoop, event: &winit::event::KeyEvent) {
            if event.state != ElementState::Pressed || event.repeat {
                return;
            }
            let Key::Character(character) = &event.logical_key else {
                return;
            };
            let lower = character.as_str().to_lowercase();
            let Some(window) = self.window.clone() else {
                return;
            };

            if lower.as_str() == "q" {
                println!("KEY_QUIT frame_pending");
                event_loop.exit();
                return;
            }

            // `v` is reserved for the normal paste shortcut. Log its origin even if modifier
            // tracking raced the key event; egui-winit itself still decides whether Ctrl+V
            // starts a transfer.
            if lower.as_str() == "v" {
                self.note_key_origin(&window, "ctrl_v".to_owned());
                return;
            }
            if self.modifiers_ctrl {
                return;
            }

            let command = match lower.as_str() {
                "i" => Some(Command::CopyImage),
                "t" => Some(Command::CopyText),
                "m" => Some(Command::ImageThenText),
                "n" => Some(Command::TextThenImage),
                _ => None,
            };
            if let Some(command) = command {
                self.pending_commands.push(command);
                self.note_key_origin(&window, format!("key={lower} cmd={}", command.label()));
            }
        }

        fn draw(&mut self, event_loop: &ActiveEventLoop) {
            let Some(window) = self.window.clone() else {
                return;
            };
            if self.state.is_none() {
                return;
            }

            self.frame += 1;
            println!("FRAME {} BEGIN", self.frame);

            let raw_input = {
                let state = self.state.as_mut().expect("state checked above");
                state.take_egui_input(window.as_ref())
            };
            self.log_raw_input(&raw_input);

            for origin in self.pending_origins.drain(..) {
                println!("FRAME {} ORIGIN {}", self.frame, origin);
            }

            let commands = core::mem::take(&mut self.pending_commands);
            let frame = self.frame;
            let mut output = self.egui_ctx.run_ui(raw_input, |ui| {
                // The software surface supplies the probe status display; no font installation
                // or selectable text is needed to exercise real platform clipboard output.
                for command in &commands {
                    match command {
                        Command::CopyImage => ui.ctx().copy_image(seed_image()),
                        Command::CopyText => ui.ctx().copy_text(TEXT.to_owned()),
                        Command::ImageThenText => {
                            ui.ctx().copy_image(seed_image());
                            ui.ctx().copy_text(TEXT.to_owned());
                        }
                        Command::TextThenImage => {
                            ui.ctx().copy_text(TEXT.to_owned());
                            ui.ctx().copy_image(seed_image());
                        }
                    }
                }
            });

            log_platform_commands(frame, &output.platform_output.commands);

            let platform_output = core::mem::take(&mut output.platform_output);
            output.drop_without_applying_deltas();
            {
                let state = self.state.as_mut().expect("state checked above");
                state.handle_platform_output_with_event_loop(
                    window.as_ref(),
                    event_loop,
                    platform_output,
                );
                println!(
                    "FRAME {} END failures={}",
                    frame,
                    state.clipboard_failure_count()
                );
                println!(
                    "FRAME {} EGUI_STATE take_egui_input=1 handle_platform_output_with_event_loop=1 window_events={}",
                    frame, self.window_events
                );
            }

            if let Some(renderer) = self.renderer.as_mut() {
                match renderer.present(frame) {
                    Ok((width, height)) => {
                        if width > 0 && height > 0 && !self.presented_once {
                            self.presented_once = true;
                            println!("FRAME {frame} PRESENTED size={width}x{height}");
                        } else if width == 0 || height == 0 {
                            window.request_redraw();
                        }
                    }
                    Err(error) => {
                        println!("FRAME {frame} PRESENT_ERROR {error}");
                        if !self.presented_once {
                            eprintln!("softbuffer presentation failed before first map: {error}");
                            event_loop.exit();
                        }
                    }
                }
            }

            if !self.mapped && self.presented_once {
                self.mapped = true;
                let size = window.inner_size();
                println!(
                    "WINDOW_MAPPED frame={} title={:?} size={}x{}",
                    frame, WINDOW_TITLE, size.width, size.height
                );
            }
        }
    }

    fn log_platform_commands(frame: u64, commands: &[OutputCommand]) {
        let mut regular = "none";
        let mut primary = false;
        for (index, command) in commands.iter().enumerate() {
            match command {
                OutputCommand::CopyText(text) => {
                    regular = "text";
                    println!(
                        "FRAME {frame} PLATFORM_COMMAND index={index} kind=CopyText bytes={} fnv={:016x} hex={}",
                        text.len(),
                        fnv1a(text.as_bytes()),
                        hex(text.as_bytes()),
                    );
                }
                OutputCommand::CopyImage(image) => {
                    regular = "image";
                    log_image("PLATFORM_IMAGE", frame, index, image);
                    println!(
                        "FRAME {frame} PLATFORM_COMMAND index={index} kind=CopyImage dims={}x{}",
                        image.size[0], image.size[1]
                    );
                }
                OutputCommand::TextSelectionSettled(text) => {
                    primary = true;
                    println!(
                        "FRAME {frame} PLATFORM_COMMAND index={index} kind=TextSelectionSettled bytes={} fnv={:016x}",
                        text.len(),
                        fnv1a(text.as_bytes()),
                    );
                }
                OutputCommand::OpenUrl(open_url) => {
                    println!(
                        "FRAME {frame} PLATFORM_COMMAND index={index} kind=OpenUrl url={:?}",
                        open_url.url
                    );
                }
            }
        }
        println!(
            "FRAME {frame} BATCH regular={regular} primary={}",
            if primary { "text" } else { "none" }
        );
    }

    impl ApplicationHandler<UserEvent> for App {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            if self.window.is_some() {
                return;
            }

            let window = match event_loop.create_window(
                Window::default_attributes()
                    .with_title(WINDOW_TITLE)
                    .with_inner_size(PhysicalSize::new(720, 480)),
            ) {
                Ok(window) => Arc::new(window),
                Err(error) => {
                    eprintln!("failed to create probe window: {error}");
                    event_loop.exit();
                    return;
                }
            };

            let renderer = match Renderer::new(&window) {
                Ok(renderer) => renderer,
                Err(error) => {
                    eprintln!("failed to initialize softbuffer: {error}");
                    event_loop.exit();
                    return;
                }
            };

            let state = egui_winit::State::new(
                self.egui_ctx.clone(),
                ViewportId::ROOT,
                window.as_ref(),
                Some(window.scale_factor() as f32),
                None,
                None,
            );

            println!(
                "STATE_READY egui_winit_state=true wayland={} viewport={:?}",
                event_loop.is_wayland(),
                ViewportId::ROOT
            );
            println!("USES_EGUI_WINIT_STATE=true");
            self.state = Some(state);
            self.renderer = Some(renderer);
            self.window = Some(Arc::clone(&window));
            window.request_redraw();
        }

        fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: UserEvent) {
            println!("USER_EVENT WAKE");
            if let Some(window) = self.window.as_ref() {
                window.request_redraw();
            }
        }

        fn window_event(
            &mut self,
            event_loop: &ActiveEventLoop,
            _window_id: WindowId,
            event: WindowEvent,
        ) {
            let Some(window) = self.window.clone() else {
                return;
            };
            let response = {
                let Some(state) = self.state.as_mut() else {
                    return;
                };
                self.window_events += 1;
                println!("WINDOW_EVENT delivered={}", self.window_events);
                state.on_window_event(window.as_ref(), &event)
            };

            let is_redraw = matches!(&event, WindowEvent::RedrawRequested);
            if response.repaint && !is_redraw {
                window.request_redraw();
            }

            match event {
                WindowEvent::ModifiersChanged(modifiers) => {
                    self.modifiers_ctrl = modifiers.state().control_key();
                    println!("MODIFIERS ctrl={}", self.modifiers_ctrl);
                }
                WindowEvent::KeyboardInput { event, .. } => {
                    self.handle_key(event_loop, &event);
                }
                WindowEvent::Focused(focused) => {
                    println!("FOCUSED {focused}");
                }
                WindowEvent::RedrawRequested => {
                    self.draw(event_loop);
                }
                WindowEvent::CloseRequested => {
                    println!("CLOSE_REQUESTED");
                    event_loop.exit();
                }
                WindowEvent::Resized(_) => {
                    window.request_redraw();
                }
                _ => {}
            }
        }

        fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
            if self.started.elapsed() > PROBE_BUDGET {
                println!(
                    "PROBE_TIMEOUT elapsed_s={}",
                    self.started.elapsed().as_secs()
                );
                event_loop.exit();
                return;
            }
            event_loop.set_control_flow(ControlFlow::WaitUntil(self.started + PROBE_BUDGET));
        }

        fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
            println!(
                "EXIT frame={} mapped={} presented_once={}",
                self.frame, self.mapped, self.presented_once
            );
        }
    }

    pub fn run() {
        let egui_ctx = egui::Context::default();
        let event_loop = EventLoop::<UserEvent>::with_user_event()
            .with_wayland()
            .build()
            .expect("build Wayland event loop");
        // `EventLoopProxy` is Send but not necessarily Sync; egui requires a Send + Sync callback.
        let proxy = Arc::new(Mutex::new(event_loop.create_proxy()));
        egui_ctx.set_request_repaint_callback(move |info| {
            println!(
                "REPAINT_CALLBACK viewport={:?} thread={:?}",
                info.viewport_id,
                std::thread::current().name()
            );
            let _ = proxy.lock().send_event(UserEvent::Wake);
        });

        let mut app = App::new(egui_ctx);
        event_loop
            .run_app(&mut app)
            .expect("run native clipboard image probe");
    }

    pub fn verify_window(path: &Path) -> Result<(), String> {
        let image = image::ImageReader::open(path)
            .map_err(|error| format!("open {}: {error}", path.display()))?
            .decode()
            .map_err(|error| format!("decode {}: {error}", path.display()))?
            .into_rgb8();
        let mut counts = [0_usize; 3];
        for pixel in image.pixels() {
            for (index, color) in [[255, 0, 0], [0, 160, 0], [0, 0, 255]].iter().enumerate() {
                if pixel.0 == *color {
                    counts[index] += 1;
                }
            }
        }
        println!(
            "WINDOW_VERIFY dims={}x{} swatch_pixels={counts:?}",
            image.width(),
            image.height()
        );
        if counts.iter().any(|count| *count < 64) {
            return Err(
                "the compositor screenshot does not contain all three painted swatches".to_owned(),
            );
        }
        Ok(())
    }

    pub fn verify_png(path: &Path) -> Result<(), String> {
        let bytes =
            std::fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
        let image = image::ImageReader::open(path)
            .map_err(|error| format!("open {}: {error}", path.display()))?
            .with_guessed_format()
            .map_err(|error| format!("guess {} format: {error}", path.display()))?
            .decode()
            .map_err(|error| format!("decode {}: {error}", path.display()))?;
        let rgba = image.into_rgba8();
        let raw = rgba.as_raw();
        let expected = seed_rgba_bytes();
        let dimensions_match = rgba.width() == 2 && rgba.height() == 2;
        let seed_ok = dimensions_match && raw.as_slice() == expected.as_slice();
        let alpha_hex = if raw.len() >= 16 {
            hex(&[raw[3], raw[7], raw[11], raw[15]])
        } else {
            "short".to_owned()
        };

        println!(
            "PNG_VERIFY path={} bytes={} dims={}x{} rgba_hex={} rgba_fnv={:016x} alpha_hex={} seed_rgba_match={}",
            path.display(),
            bytes.len(),
            rgba.width(),
            rgba.height(),
            hex(raw),
            fnv1a(raw),
            alpha_hex,
            seed_ok,
        );

        if !seed_ok {
            return Err(format!(
                "decoded PNG does not match the 2x2 seed (dims {}x{}, rgba {})",
                rgba.width(),
                rgba.height(),
                hex(raw),
            ));
        }
        Ok(())
    }
}

#[cfg(all(
    target_os = "linux",
    feature = "wayland",
    feature = "clipboard",
    feature = "clipboard-image"
))]
fn main() {
    let mut arguments = std::env::args_os().skip(1);
    match arguments.next() {
        None => probe::run(),
        Some(flag) if matches!(flag.to_str(), Some("--verify-png" | "--verify-window")) => {
            let Some(path) = arguments.next() else {
                eprintln!("usage: clipboard_image_probe --verify-png|--verify-window FILE");
                std::process::exit(2);
            };
            if arguments.next().is_some() {
                eprintln!("usage: clipboard_image_probe --verify-png|--verify-window FILE");
                std::process::exit(2);
            }
            let result = if flag.to_str() == Some("--verify-window") {
                probe::verify_window(std::path::Path::new(&path))
            } else {
                probe::verify_png(std::path::Path::new(&path))
            };
            if let Err(error) = result {
                eprintln!("PNG_VERIFY_ERROR {error}");
                std::process::exit(2);
            }
        }
        Some(flag) if matches!(flag.to_str(), Some("--help" | "-h")) => {
            println!("usage: clipboard_image_probe [--verify-png|--verify-window FILE]");
            println!("Normal mode requires a private Wayland compositor in XDG_RUNTIME_DIR.");
        }
        Some(flag) => {
            eprintln!("unknown argument: {}", flag.display());
            eprintln!("usage: clipboard_image_probe [--verify-png|--verify-window FILE]");
            std::process::exit(2);
        }
    }
}

#[cfg(not(all(
    target_os = "linux",
    feature = "wayland",
    feature = "clipboard",
    feature = "clipboard-image"
)))]
fn main() {
    eprintln!(
        "clipboard_image_probe requires Linux Wayland with egui-winit features \
         'wayland', 'clipboard', and 'clipboard-image'"
    );
    std::process::exit(2);
}
