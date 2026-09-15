//! The emulator application: game screen, input, audio pump and the
//! built-in debugger (registers, disassembly, memory, breakpoints, trace).

use eframe::egui;
use gb_core::cpu::registers::{FLAG_C, FLAG_H, FLAG_N, FLAG_Z};
use gb_core::disasm::disassemble;
use gb_core::{Button, Cartridge, GameBoy, SCREEN_H, SCREEN_W};

use crate::audio::Audio;
use crate::pacing::{Clock, Pacer, GB_FPS};
use crate::screen::{ScreenCallback, ScreenRenderer};

pub struct EmulatorApp {
    gb: Option<GameBoy>,
    audio: Audio,
    prev_frame: Vec<u8>,
    cur_frame: Vec<u8>,
    show_debugger: bool,
    ghosting: f32,
    grid: f32,
    mem_addr: String,
    mem_view_base: u16,
    bp_input: String,
    status: String,
    follow_pc: bool,
    disasm_base: u16,
    last_update: Option<std::time::Instant>,
    pacer: Pacer,
    /// Pending result from the async ROM-picker dialog.
    rom_rx: Option<std::sync::mpsc::Receiver<Option<std::path::PathBuf>>>,
}

impl EmulatorApp {
    pub fn new(cc: &eframe::CreationContext<'_>, rom_path: Option<String>) -> Self {
        // Register the custom wgpu pipeline for the screen.
        let render_state = cc
            .wgpu_render_state
            .as_ref()
            .expect("wgpu backend required (eframe was built with the wgpu feature)");
        render_state
            .renderer
            .write()
            .callback_resources
            .insert(ScreenRenderer::new(
                &render_state.device,
                render_state.target_format,
            ));

        let audio = Audio::new();
        let pacer = Pacer::new(audio.sample_rate);
        let mut app = EmulatorApp {
            gb: None,
            audio,
            pacer,
            prev_frame: vec![0; SCREEN_W * SCREEN_H],
            cur_frame: vec![0; SCREEN_W * SCREEN_H],
            show_debugger: true,
            ghosting: 0.35,
            grid: 0.6,
            mem_addr: "C000".into(),
            mem_view_base: 0xC000,
            bp_input: String::new(),
            status: "Load a ROM to start (File > Open, or drop a .gb file)".into(),
            follow_pc: true,
            disasm_base: 0x0100,
            last_update: None,
            rom_rx: None,
        };
        if let Some(path) = rom_path {
            app.load_rom_path(&path);
        }
        app
    }

    fn load_rom_path(&mut self, path: &str) {
        match std::fs::read(path) {
            Ok(bytes) => self.load_rom_bytes(bytes),
            Err(e) => self.status = format!("Failed to read {path}: {e}"),
        }
    }

    fn load_rom_bytes(&mut self, bytes: Vec<u8>) {
        match Cartridge::from_rom(bytes) {
            Ok(cart) => {
                self.status = format!("Running: {}", cart.title);
                let mut gb = GameBoy::new(cart);
                gb.bus.apu.set_sample_rate(self.audio.sample_rate);
                self.gb = Some(gb);
            }
            Err(e) => self.status = format!("Bad ROM: {e}"),
        }
    }

    fn handle_input(&mut self, ctx: &egui::Context) {
        let Some(gb) = self.gb.as_mut() else { return };

        // Debug auto-navigation: tap Start until the game reports in-game,
        // so audio in a real stage can be measured without a windowing setup.
        if std::env::var_os("GBEM_AUTOPLAY").is_some() && gb.bus.read(0xFFE1) != 0 {
            let pressed = (gb.cycles / 70224) % 60 < 5;
            gb.set_button(Button::Start, pressed);
            return;
        }

        ctx.input(|i| {
            let map = [
                (egui::Key::ArrowUp, Button::Up),
                (egui::Key::ArrowDown, Button::Down),
                (egui::Key::ArrowLeft, Button::Left),
                (egui::Key::ArrowRight, Button::Right),
                (egui::Key::Z, Button::A),
                (egui::Key::X, Button::B),
                (egui::Key::Enter, Button::Start),
                (egui::Key::Backspace, Button::Select),
            ];
            for (key, btn) in map {
                if i.key_pressed(key) {
                    gb.set_button(btn, true);
                }
                if i.key_released(key) {
                    gb.set_button(btn, false);
                }
            }
            if i.key_pressed(egui::Key::P) {
                gb.toggle_pause();
            }
            if i.key_pressed(egui::Key::N) && gb.is_paused() {
                gb.step_instruction();
            }
        });
    }

    fn run_emulation(&mut self) {
        let Some(gb) = self.gb.as_mut() else { return };
        if gb.is_paused() {
            self.last_update = None;
            self.pacer.reset();
            // Keep the debugger view of the framebuffer fresh while stepping.
            self.cur_frame.copy_from_slice(gb.framebuffer());
            return;
        }

        // Read the clock and let the pacer decide how many frames to run. When
        // a sound device is open it is the clock — topping the queue up to a
        // target depth keeps true speed and never drops or gaps samples (a
        // dropped chunk is audible as broken music in a stage). Otherwise we
        // fall back to wall-clock time.
        let frames = if self.audio.available {
            self.pacer
                .frames_due(Clock::Audio { queued_frames: self.audio.queued_frames() })
        } else {
            let now = std::time::Instant::now();
            let elapsed = self
                .last_update
                .map(|t| now - t)
                .unwrap_or_else(|| std::time::Duration::from_secs_f64(1.0 / GB_FPS));
            self.last_update = Some(now);
            self.pacer.frames_due(Clock::Wall { elapsed })
        };

        for _ in 0..frames {
            if let Some(stop) = gb.run_frame() {
                self.status = format!("Stopped: {stop:?}");
                break;
            }
            let samples = gb.bus.apu.drain_samples();
            if self.audio.available {
                self.audio.push_samples(&samples);
            }
            self.prev_frame.copy_from_slice(&self.cur_frame);
            self.cur_frame.copy_from_slice(gb.framebuffer());
        }
    }

    fn screen_ui(&mut self, ui: &mut egui::Ui) {
        let avail = ui.available_size();
        let aspect = SCREEN_W as f32 / SCREEN_H as f32;
        let size = if avail.x / avail.y > aspect {
            egui::vec2(avail.y * aspect, avail.y)
        } else {
            egui::vec2(avail.x, avail.x / aspect)
        };
        let (rect, _) =
            ui.allocate_exact_size(size, egui::Sense::focusable_noninteractive());
        ui.painter().add(egui_wgpu_callback(
            rect,
            ScreenCallback {
                current: self.cur_frame.clone(),
                previous: self.prev_frame.clone(),
                ghosting: self.ghosting,
                grid: self.grid,
            },
        ));
    }

    fn debugger_ui(&mut self, ctx: &egui::Context) {
        let Some(gb) = self.gb.as_mut() else { return };

        egui::SidePanel::right("debugger")
            .default_width(430.0)
            .show(ctx, |ui| {
                ui.heading("Debugger");
                ui.horizontal(|ui| {
                    let paused = gb.is_paused();
                    let label = if paused { "▶ Continue (P)" } else { "⏸ Pause (P)" };
                    if ui.button(label).clicked() {
                        gb.toggle_pause();
                    }
                    if ui
                        .add_enabled(paused, egui::Button::new("Step (N)"))
                        .clicked()
                    {
                        gb.step_instruction();
                    }
                    if ui
                        .add_enabled(paused, egui::Button::new("Step frame"))
                        .clicked()
                    {
                        gb.step_frame();
                    }
                });
                ui.separator();

                // Registers
                let r = &gb.cpu.regs;
                ui.monospace(format!(
                    "AF {:04X}  BC {:04X}  DE {:04X}  HL {:04X}",
                    r.af(),
                    r.bc(),
                    r.de(),
                    r.hl()
                ));
                ui.monospace(format!(
                    "PC {:04X}  SP {:04X}  IME {}  cycles {}",
                    r.pc,
                    r.sp,
                    if gb.cpu.ime { "on " } else { "off" },
                    gb.cycles
                ));
                ui.monospace(format!(
                    "flags [{}{}{}{}]   halted: {}",
                    if r.flag(FLAG_Z) { 'Z' } else { '-' },
                    if r.flag(FLAG_N) { 'N' } else { '-' },
                    if r.flag(FLAG_H) { 'H' } else { '-' },
                    if r.flag(FLAG_C) { 'C' } else { '-' },
                    gb.cpu.halted,
                ));
                ui.monospace(format!(
                    "LCDC {:02X}  STAT {:02X}  LY {:3}  IE {:02X}  IF {:02X}",
                    gb.bus.ppu.lcdc,
                    gb.bus.ppu.stat,
                    gb.bus.ppu.ly,
                    gb.bus.ints.enable,
                    gb.bus.ints.request
                ));
                ui.separator();

                // Disassembly
                ui.horizontal(|ui| {
                    ui.label("Disassembly");
                    ui.checkbox(&mut self.follow_pc, "follow PC");
                });
                if self.follow_pc {
                    self.disasm_base = gb.cpu.regs.pc;
                }
                let mut addr = self.disasm_base;
                egui::ScrollArea::vertical()
                    .id_salt("disasm")
                    .max_height(220.0)
                    .show(ui, |ui| {
                        for _ in 0..24 {
                            let (text, len) = disassemble(|a| gb.bus.read(a), addr);
                            let is_pc = addr == gb.cpu.regs.pc;
                            let has_bp = gb.has_breakpoint(addr);
                            let marker = match (has_bp, is_pc) {
                                (true, true) => "●▶",
                                (true, false) => "● ",
                                (false, true) => " ▶",
                                (false, false) => "  ",
                            };
                            let line = format!("{marker} {addr:04X}  {text}");
                            let resp = ui.selectable_label(
                                is_pc,
                                egui::RichText::new(line).monospace().color(if has_bp {
                                    egui::Color32::from_rgb(230, 80, 80)
                                } else if is_pc {
                                    egui::Color32::from_rgb(120, 220, 120)
                                } else {
                                    egui::Color32::GRAY
                                }),
                            );
                            if resp.clicked() {
                                gb.toggle_breakpoint(addr);
                            }
                            addr = addr.wrapping_add(len);
                        }
                    });

                // Breakpoints
                ui.horizontal(|ui| {
                    ui.label("Breakpoint (hex):");
                    ui.add(egui::TextEdit::singleline(&mut self.bp_input).desired_width(60.0));
                    if ui.button("Add/Remove").clicked() {
                        if let Ok(a) = u16::from_str_radix(self.bp_input.trim_start_matches("0x"), 16)
                        {
                            gb.toggle_breakpoint(a);
                        }
                    }
                });
                let bps = gb.breakpoints();
                if !bps.is_empty() {
                    let list = bps
                        .iter()
                        .map(|b| format!("{b:04X}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    ui.monospace(format!("breakpoints: {list}"));
                }
                ui.separator();

                // Memory viewer
                ui.horizontal(|ui| {
                    ui.label("Memory @");
                    let resp =
                        ui.add(egui::TextEdit::singleline(&mut self.mem_addr).desired_width(60.0));
                    if resp.lost_focus() || ui.button("Go").clicked() {
                        if let Ok(a) =
                            u16::from_str_radix(self.mem_addr.trim_start_matches("0x"), 16)
                        {
                            self.mem_view_base = a & 0xFFF0;
                        }
                    }
                });
                egui::ScrollArea::vertical()
                    .id_salt("mem")
                    .max_height(160.0)
                    .show(ui, |ui| {
                        for row in 0..10u16 {
                            let base = self.mem_view_base.wrapping_add(row * 16);
                            let bytes: Vec<String> = (0..16)
                                .map(|i| format!("{:02X}", gb.bus.read(base.wrapping_add(i))))
                                .collect();
                            let ascii: String = (0..16)
                                .map(|i| {
                                    let b = gb.bus.read(base.wrapping_add(i));
                                    if b.is_ascii_graphic() { b as char } else { '.' }
                                })
                                .collect();
                            ui.monospace(format!("{base:04X}  {}  {ascii}", bytes.join(" ")));
                        }
                    });
                ui.separator();

                // Execution trace
                ui.collapsing("Execution trace (last 16)", |ui| {
                    let trace = gb.trace();
                    for pc in trace.iter().rev().take(16) {
                        let (text, _) = disassemble(|a| gb.bus.read(a), *pc);
                        ui.monospace(format!("{pc:04X}  {text}"));
                    }
                });

                // Shader controls
                ui.separator();
                ui.label("LCD shader");
                ui.add(egui::Slider::new(&mut self.ghosting, 0.0..=0.9).text("ghosting"));
                ui.add(egui::Slider::new(&mut self.grid, 0.0..=1.0).text("pixel grid"));
            });
    }
}

fn egui_wgpu_callback(
    rect: egui::Rect,
    callback: ScreenCallback,
) -> egui::Shape {
    egui::Shape::Callback(eframe::egui_wgpu::Callback::new_paint_callback(
        rect, callback,
    ))
}

impl eframe::App for EmulatorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Async file-picker result.
        if let Some(rx) = &self.rom_rx {
            match rx.try_recv() {
                Ok(Some(path)) => {
                    self.rom_rx = None;
                    self.load_rom_path(&path.display().to_string());
                }
                Ok(None) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.rom_rx = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }

        // Drag & drop ROM loading.
        let dropped: Vec<_> = ctx.input(|i| i.raw.dropped_files.clone());
        for file in dropped {
            if let Some(path) = file.path {
                self.load_rom_path(&path.display().to_string());
            } else if let Some(bytes) = file.bytes {
                self.load_rom_bytes(bytes.to_vec());
            }
        }

        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open ROM…").clicked() {
                        // Never run the modal dialog on the main thread: on
                        // macOS its nested event loop can stop winit's loop
                        // and silently quit the app.
                        let (tx, rx) = std::sync::mpsc::channel();
                        self.rom_rx = Some(rx);
                        std::thread::spawn(move || {
                            let picked = pollster::block_on(
                                rfd::AsyncFileDialog::new()
                                    .add_filter("Game Boy ROM", &["gb", "gbc", "bin"])
                                    .pick_file(),
                            )
                            .map(|h| h.path().to_path_buf());
                            let _ = tx.send(picked);
                        });
                        ui.close();
                    }
                });
                ui.menu_button("View", |ui| {
                    ui.checkbox(&mut self.show_debugger, "Debugger");
                });
                ui.label(&self.status);
            });
        });

        self.handle_input(ctx);
        self.run_emulation();

        if self.show_debugger {
            self.debugger_ui(ctx);
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(egui::Color32::from_rgb(20, 24, 18)))
            .show(ctx, |ui| {
                ui.centered_and_justified(|ui| {
                    self.screen_ui(ui);
                });
            });

        ctx.request_repaint();
    }
}
