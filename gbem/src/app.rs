//! The emulator application: menu bar, game screen, collapsible sidebar
//! (debugger / video / controls / audio), a bottom status bar, and a Settings
//! modal for remappable controls. Clean-studio dark theme with an indigo accent.

use std::path::{Path, PathBuf};

use eframe::egui;
use gb_core::cpu::registers::{FLAG_C, FLAG_H, FLAG_N, FLAG_Z};
use gb_core::disasm::disassemble;
use gb_core::{Button, Cartridge, GameBoy, SCREEN_H, SCREEN_W};

use crate::audio::Audio;
use crate::config::{self, Controls, BUTTONS};
use crate::gamepad::Gamepad;
use crate::pacing::{Clock, Pacer, GB_FPS};
use crate::saves;
use crate::screen::{ScreenCallback, ScreenRenderer};

/// Flush battery RAM this long after the last write settles.
const AUTOSAVE_DEBOUNCE: std::time::Duration = std::time::Duration::from_secs(2);
const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x7c, 0x83, 0xff);

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Controls,
    Video,
    Audio,
}

/// Which binding is currently being reassigned.
#[derive(Clone, Copy)]
enum Rebind {
    Key(usize),
    Pad(usize),
}

pub struct EmulatorApp {
    gb: Option<GameBoy>,
    audio: Audio,
    gamepad: Gamepad,
    controls: Controls,
    prev_frame: Vec<u8>,
    cur_frame: Vec<u8>,
    show_sidebar: bool,
    settings_open: bool,
    settings_tab: Tab,
    rebind: Option<Rebind>,
    ghosting: f32,
    grid: f32,
    volume: f32,
    mem_addr: String,
    mem_view_base: u16,
    bp_input: String,
    status: String,
    follow_pc: bool,
    disasm_base: u16,
    last_update: Option<std::time::Instant>,
    pacer: Pacer,
    // Live emulation speed (emulated frames per real second).
    fps: u32,
    fps_count: u32,
    fps_since: std::time::Instant,
    save_path: Option<PathBuf>,
    unsaved_ram: bool,
    last_ram_change: Option<std::time::Instant>,
    save_requested: bool,
    state_path: Option<PathBuf>,
    state_save_requested: bool,
    state_load_requested: bool,
    rom_rx: Option<std::sync::mpsc::Receiver<Option<PathBuf>>>,
}

impl EmulatorApp {
    pub fn new(cc: &eframe::CreationContext<'_>, rom_path: Option<String>) -> Self {
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

        apply_theme(&cc.egui_ctx);

        let audio = Audio::new();
        let pacer = Pacer::new(audio.sample_rate);
        let mut app = EmulatorApp {
            gb: None,
            audio,
            gamepad: Gamepad::new(),
            controls: Controls::load(),
            pacer,
            prev_frame: vec![0; SCREEN_W * SCREEN_H * 4],
            cur_frame: vec![0; SCREEN_W * SCREEN_H * 4],
            show_sidebar: true,
            settings_open: false,
            settings_tab: Tab::Controls,
            rebind: None,
            ghosting: 0.35,
            grid: 0.6,
            volume: 0.8,
            mem_addr: "C000".into(),
            mem_view_base: 0xC000,
            bp_input: String::new(),
            status: "Drop a .gb / .gbc ROM here, or use File → Open ROM…".into(),
            follow_pc: true,
            disasm_base: 0x0100,
            last_update: None,
            fps: 0,
            fps_count: 0,
            fps_since: std::time::Instant::now(),
            save_path: None,
            unsaved_ram: false,
            last_ram_change: None,
            save_requested: false,
            state_path: None,
            state_save_requested: false,
            state_load_requested: false,
            rom_rx: None,
        };
        if let Some(path) = rom_path {
            app.load_rom_path(Path::new(&path));
        }
        app
    }

    // ---- ROM & persistence (unchanged behaviour) ----

    fn load_rom_path(&mut self, path: &Path) {
        match std::fs::read(path) {
            Ok(bytes) => self.load_rom_bytes(bytes, Some(path.to_path_buf())),
            Err(e) => self.status = format!("Failed to read {}: {e}", path.display()),
        }
    }

    fn load_rom_bytes(&mut self, bytes: Vec<u8>, path: Option<PathBuf>) {
        self.flush_save();
        match Cartridge::from_rom(bytes) {
            Ok(mut cart) => {
                self.status = format!("Running: {}", cart.title);
                self.save_path = match (&path, cart.has_battery()) {
                    (Some(p), true) => Some(saves::save_path_for(p)),
                    _ => None,
                };
                self.state_path = path.as_deref().map(saves::state_path_for);
                if let Some(sav) = &self.save_path {
                    if let Some(data) = saves::load(sav) {
                        cart.load_ram(&data);
                    }
                }
                self.unsaved_ram = false;
                self.last_ram_change = None;
                let mut gb = GameBoy::new(cart);
                gb.bus.apu.set_sample_rate(self.audio.sample_rate);
                self.gb = Some(gb);
            }
            Err(e) => self.status = format!("Bad ROM: {e}"),
        }
    }

    fn autosave_tick(&mut self) {
        let now = std::time::Instant::now();
        let dirty = match self.gb.as_mut() {
            Some(gb) => gb.bus.cart.take_ram_dirty(),
            None => return,
        };
        if dirty {
            self.unsaved_ram = true;
            self.last_ram_change = Some(now);
        }
        if self.unsaved_ram {
            if let Some(t) = self.last_ram_change {
                if now.duration_since(t) >= AUTOSAVE_DEBOUNCE {
                    self.flush_save();
                }
            }
        }
    }

    fn flush_save(&mut self) {
        if !self.unsaved_ram {
            return;
        }
        let Some(path) = self.save_path.clone() else { return };
        let Some(gb) = self.gb.as_ref() else { return };
        match saves::write(&path, gb.bus.cart.ram()) {
            Ok(()) => self.unsaved_ram = false,
            Err(e) => self.status = format!("Save failed: {e}"),
        }
    }

    fn save_state_file(&mut self) {
        let Some(path) = self.state_path.clone() else {
            self.status = "Load a ROM from a file to use save states".into();
            return;
        };
        let Some(gb) = self.gb.as_ref() else { return };
        let data = gb.save_state();
        match saves::write(&path, &data) {
            Ok(()) => self.status = "Saved state".into(),
            Err(e) => self.status = format!("Save state failed: {e}"),
        }
    }

    fn load_state_file(&mut self) {
        let Some(path) = self.state_path.clone() else { return };
        let Some(data) = saves::load(&path) else {
            self.status = "No save state for this ROM".into();
            return;
        };
        let Some(gb) = self.gb.as_mut() else { return };
        match gb.load_state(&data) {
            Ok(()) => self.status = "Loaded save state".into(),
            Err(e) => self.status = format!("Load state failed: {e}"),
        }
    }

    fn save_now(&mut self) {
        let Some(path) = self.save_path.clone() else {
            self.status = "This ROM has no battery save".into();
            return;
        };
        let Some(gb) = self.gb.as_ref() else { return };
        match saves::write(&path, gb.bus.cart.ram()) {
            Ok(()) => {
                self.unsaved_ram = false;
                self.status = "Saved battery RAM".into();
            }
            Err(e) => self.status = format!("Save failed: {e}"),
        }
    }

    fn open_rom_dialog(&mut self) {
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
    }

    // ---- input ----

    fn gameplay_input(&mut self, ctx: &egui::Context, pad: [bool; 8]) {
        let Some(gb) = self.gb.as_mut() else { return };

        if std::env::var_os("GBEM_AUTOPLAY").is_some() && gb.bus.read(0xFFE1) != 0 {
            let pressed = (gb.cycles / 70224) % 60 < 5;
            gb.set_button(Button::Start, pressed);
            return;
        }

        ctx.input(|i| {
            for (idx, (_, btn)) in BUTTONS.iter().enumerate() {
                let held = i.key_down(self.controls.keys[idx]) || pad[idx];
                gb.set_button(*btn, held);
            }
            if i.key_pressed(egui::Key::P) {
                gb.toggle_pause();
            }
            if i.key_pressed(egui::Key::N) && gb.is_paused() {
                gb.step_instruction();
            }
            if i.key_pressed(egui::Key::F5) {
                self.state_save_requested = true;
            }
            if i.key_pressed(egui::Key::F9) {
                self.state_load_requested = true;
            }
        });
    }

    /// Capture a fresh key/pad press to complete a rebinding.
    fn capture_rebind(&mut self, ctx: &egui::Context) {
        let Some(rebind) = self.rebind else { return };
        match rebind {
            Rebind::Key(i) => {
                let key = ctx.input(|inp| {
                    inp.events.iter().find_map(|e| match e {
                        egui::Event::Key { key, pressed: true, .. } => Some(*key),
                        _ => None,
                    })
                });
                if let Some(k) = key {
                    if k != egui::Key::Escape {
                        self.controls.keys[i] = k;
                        self.controls.save();
                    }
                    self.rebind = None;
                }
            }
            Rebind::Pad(i) => {
                if let Some(b) = self.gamepad.take_captured() {
                    self.controls.pads[i] = b;
                    self.controls.save();
                    self.rebind = None;
                }
            }
        }
    }

    // ---- emulation ----

    fn run_emulation(&mut self) {
        let Some(gb) = self.gb.as_mut() else { return };
        if gb.is_paused() {
            self.last_update = None;
            self.pacer.reset();
            self.cur_frame.copy_from_slice(gb.framebuffer());
            return;
        }
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
            let mut samples = gb.bus.apu.drain_samples();
            if self.audio.available {
                if self.volume != 1.0 {
                    for s in &mut samples {
                        *s *= self.volume;
                    }
                }
                self.audio.push_samples(&samples);
            }
            self.prev_frame.copy_from_slice(&self.cur_frame);
            self.cur_frame.copy_from_slice(gb.framebuffer());
            self.fps_count += 1;
        }

        if self.fps_since.elapsed() >= std::time::Duration::from_secs(1) {
            self.fps = self.fps_count;
            self.fps_count = 0;
            self.fps_since = std::time::Instant::now();
        }
    }

    // ---- screen ----

    fn screen_ui(&mut self, ui: &mut egui::Ui) {
        let avail = ui.available_size();
        if self.gb.is_none() {
            ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new("Drop a ROM here\nor  File → Open ROM…")
                        .size(18.0)
                        .color(egui::Color32::from_gray(120)),
                );
            });
            return;
        }
        let aspect = SCREEN_W as f32 / SCREEN_H as f32;
        // Leave room for the bezel padding.
        let pad = 14.0;
        let inner = egui::vec2(avail.x - pad * 2.0, avail.y - pad * 2.0);
        let size = if inner.x / inner.y > aspect {
            egui::vec2(inner.y * aspect, inner.y)
        } else {
            egui::vec2(inner.x, inner.x / aspect)
        };
        ui.centered_and_justified(|ui| {
            egui::Frame::NONE
                .fill(egui::Color32::from_rgb(0x1b, 0x1f, 0x2b))
                .inner_margin(egui::Margin::same(pad as i8))
                .corner_radius(egui::CornerRadius::same(10))
                .show(ui, |ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(size, egui::Sense::focusable_noninteractive());
                    ui.painter().add(egui::Shape::Callback(
                        eframe::egui_wgpu::Callback::new_paint_callback(
                            rect,
                            ScreenCallback {
                                current: self.cur_frame.clone(),
                                previous: self.prev_frame.clone(),
                                ghosting: self.ghosting,
                                grid: self.grid,
                            },
                        ),
                    ));
                });
        });
    }

    // ---- sidebar ----

    fn sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::right("sidebar")
            .default_width(300.0)
            .show(ctx, |ui| {
                egui::CollapsingHeader::new(egui::RichText::new("Debugger").strong())
                    .default_open(false)
                    .show(ui, |ui| self.debugger_section(ui));

                egui::CollapsingHeader::new(egui::RichText::new("Video").strong())
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.add(egui::Slider::new(&mut self.ghosting, 0.0f32..=0.9_f32).text("Ghosting"));
                        ui.add(egui::Slider::new(&mut self.grid, 0.0f32..=1.0_f32).text("Pixel grid"));
                    });

                egui::CollapsingHeader::new(egui::RichText::new("Controls").strong())
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if ui
                                .add(egui::Button::new("Configure…").fill(ACCENT))
                                .clicked()
                            {
                                self.settings_open = true;
                                self.settings_tab = Tab::Controls;
                            }
                            if ui.button("Reset").clicked() {
                                self.controls = Controls::default();
                                self.controls.save();
                            }
                        });
                        match self.gamepad.name() {
                            Some(n) => ui.label(format!("🎮 {n}")),
                            None => ui.label(
                                egui::RichText::new("No controller")
                                    .color(egui::Color32::from_gray(120)),
                            ),
                        };
                    });

                egui::CollapsingHeader::new(egui::RichText::new("Audio").strong())
                    .default_open(false)
                    .show(ui, |ui| {
                        ui.add(egui::Slider::new(&mut self.volume, 0.0f32..=1.0_f32).text("Volume"));
                    });
            });
    }

    fn debugger_section(&mut self, ui: &mut egui::Ui) {
        let Some(gb) = self.gb.as_mut() else {
            ui.label("No ROM loaded.");
            return;
        };
        ui.horizontal(|ui| {
            let paused = gb.is_paused();
            if ui.button(if paused { "▶ Run" } else { "⏸ Pause" }).clicked() {
                gb.toggle_pause();
            }
            if ui.add_enabled(paused, egui::Button::new("Step")).clicked() {
                gb.step_instruction();
            }
            if ui.add_enabled(paused, egui::Button::new("Frame")).clicked() {
                gb.step_frame();
            }
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(self.save_path.is_some(), egui::Button::new("💾 Save RAM"))
                .clicked()
            {
                self.save_requested = true;
            }
            if ui
                .add_enabled(self.state_path.is_some(), egui::Button::new("Save state"))
                .clicked()
            {
                self.state_save_requested = true;
            }
            if ui
                .add_enabled(self.state_path.is_some(), egui::Button::new("Load state"))
                .clicked()
            {
                self.state_load_requested = true;
            }
        });
        ui.separator();

        let r = &gb.cpu.regs;
        ui.monospace(format!(
            "AF {:04X}  BC {:04X}  DE {:04X}  HL {:04X}",
            r.af(),
            r.bc(),
            r.de(),
            r.hl()
        ));
        ui.monospace(format!(
            "PC {:04X}  SP {:04X}  IME {}",
            r.pc,
            r.sp,
            if gb.cpu.ime { "on" } else { "off" }
        ));
        ui.monospace(format!(
            "flags [{}{}{}{}]  halted {}",
            if r.flag(FLAG_Z) { 'Z' } else { '-' },
            if r.flag(FLAG_N) { 'N' } else { '-' },
            if r.flag(FLAG_H) { 'H' } else { '-' },
            if r.flag(FLAG_C) { 'C' } else { '-' },
            gb.cpu.halted,
        ));
        ui.monospace(format!(
            "LCDC {:02X}  STAT {:02X}  LY {:3}",
            gb.bus.ppu.lcdc, gb.bus.ppu.stat, gb.bus.ppu.ly
        ));
        ui.separator();

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
            .max_height(180.0)
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
                    let color = if has_bp {
                        egui::Color32::from_rgb(230, 80, 80)
                    } else if is_pc {
                        ACCENT
                    } else {
                        egui::Color32::GRAY
                    };
                    if ui
                        .selectable_label(
                            is_pc,
                            egui::RichText::new(format!("{marker} {addr:04X}  {text}"))
                                .monospace()
                                .color(color),
                        )
                        .clicked()
                    {
                        gb.toggle_breakpoint(addr);
                    }
                    addr = addr.wrapping_add(len);
                }
            });

        ui.horizontal(|ui| {
            ui.label("Breakpoint");
            ui.add(egui::TextEdit::singleline(&mut self.bp_input).desired_width(56.0));
            if ui.button("±").clicked() {
                if let Ok(a) = u16::from_str_radix(self.bp_input.trim_start_matches("0x"), 16) {
                    gb.toggle_breakpoint(a);
                }
            }
        });
        ui.separator();

        ui.horizontal(|ui| {
            ui.label("Memory @");
            let resp =
                ui.add(egui::TextEdit::singleline(&mut self.mem_addr).desired_width(56.0));
            if resp.lost_focus() || ui.button("Go").clicked() {
                if let Ok(a) = u16::from_str_radix(self.mem_addr.trim_start_matches("0x"), 16) {
                    self.mem_view_base = a & 0xFFF0;
                }
            }
        });
        egui::ScrollArea::vertical()
            .id_salt("mem")
            .max_height(150.0)
            .show(ui, |ui| {
                for row in 0..12u16 {
                    let base = self.mem_view_base.wrapping_add(row * 16);
                    let bytes: Vec<String> = (0..16)
                        .map(|i| format!("{:02X}", gb.bus.read(base.wrapping_add(i))))
                        .collect();
                    ui.monospace(format!("{base:04X}  {}", bytes.join(" ")));
                }
            });
    }

    // ---- settings modal ----

    fn settings_modal(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let modal = egui::Modal::new(egui::Id::new("settings")).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.horizontal(|ui| {
                ui.heading("Settings");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new("Esc to close").color(egui::Color32::from_gray(120)),
                    );
                });
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.settings_tab, Tab::Controls, "Controls");
                ui.selectable_value(&mut self.settings_tab, Tab::Video, "Video");
                ui.selectable_value(&mut self.settings_tab, Tab::Audio, "Audio");
            });
            ui.separator();

            match self.settings_tab {
                Tab::Controls => self.controls_tab(ui),
                Tab::Video => {
                    ui.add(egui::Slider::new(&mut self.ghosting, 0.0f32..=0.9_f32).text("Ghosting"));
                    ui.add(egui::Slider::new(&mut self.grid, 0.0f32..=1.0_f32).text("Pixel grid"));
                }
                Tab::Audio => {
                    ui.add(egui::Slider::new(&mut self.volume, 0.0f32..=1.0_f32).text("Volume"));
                }
            }

            ui.separator();
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Bindings save automatically.")
                        .color(egui::Color32::from_gray(120)),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::Button::new("Done").fill(ACCENT)).clicked() {
                        self.settings_open = false;
                        self.rebind = None;
                    }
                    if self.settings_tab == Tab::Controls && ui.button("Reset to defaults").clicked()
                    {
                        self.controls = Controls::default();
                        self.controls.save();
                    }
                });
            });
        });
        if modal.should_close() {
            self.settings_open = false;
            self.rebind = None;
        }
    }

    fn controls_tab(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("bindings")
            .num_columns(3)
            .spacing([16.0, 8.0])
            .show(ui, |ui| {
                ui.label(egui::RichText::new("Button").color(egui::Color32::from_gray(140)));
                ui.label(egui::RichText::new("Keyboard").color(egui::Color32::from_gray(140)));
                ui.label(egui::RichText::new("Controller").color(egui::Color32::from_gray(140)));
                ui.end_row();

                for (idx, (name, _)) in BUTTONS.iter().enumerate() {
                    ui.label(egui::RichText::new(*name).strong());

                    let key_listening = matches!(self.rebind, Some(Rebind::Key(i)) if i == idx);
                    let key_label = if key_listening {
                        "press a key…".to_string()
                    } else {
                        self.controls.keys[idx].name().to_string()
                    };
                    if ui
                        .add(bind_button(key_label, key_listening))
                        .clicked()
                    {
                        self.rebind = Some(Rebind::Key(idx));
                    }

                    let pad_listening = matches!(self.rebind, Some(Rebind::Pad(i)) if i == idx);
                    let pad_label = if pad_listening {
                        "press a button…".to_string()
                    } else {
                        config::pad_name(self.controls.pads[idx]).to_string()
                    };
                    if ui
                        .add(bind_button(pad_label, pad_listening))
                        .clicked()
                    {
                        self.rebind = Some(Rebind::Pad(idx));
                    }
                    ui.end_row();
                }
            });
    }

    // ---- status bar ----

    fn status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if let Some(gb) = self.gb.as_ref() {
                    let (glyph, col) = if gb.is_paused() {
                        ("⏸", egui::Color32::from_gray(150))
                    } else {
                        ("●", ACCENT)
                    };
                    ui.label(egui::RichText::new(glyph).color(col));
                    ui.label(egui::RichText::new(&gb.bus.cart.title).strong());
                    ui.separator();
                    ui.label(if gb.bus.cgb { "CGB" } else { "DMG" });
                    ui.separator();
                    ui.label(format!("{} fps", self.fps));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(&self.status).color(egui::Color32::from_gray(150)),
                    );
                });
            });
        });
    }
}

fn bind_button(label: String, listening: bool) -> egui::Button<'static> {
    let mut text = egui::RichText::new(label).monospace();
    if listening {
        text = text.color(ACCENT);
    }
    let mut b = egui::Button::new(text).min_size(egui::vec2(120.0, 0.0));
    if listening {
        b = b.stroke(egui::Stroke::new(1.5f32, ACCENT));
    }
    b
}

fn apply_theme(ctx: &egui::Context) {
    use egui::Color32;
    let mut v = egui::Visuals::dark();
    v.panel_fill = Color32::from_rgb(0x15, 0x18, 0x21);
    v.window_fill = Color32::from_rgb(0x17, 0x1b, 0x26);
    v.window_stroke = egui::Stroke::new(1.0f32, Color32::from_rgb(0x2a, 0x30, 0x40));
    v.extreme_bg_color = Color32::from_rgb(0x0e, 0x11, 0x18);
    v.selection.bg_fill = Color32::from_rgb(0x2e, 0x33, 0x66);
    v.selection.stroke = egui::Stroke::new(1.0f32, ACCENT);
    v.hyperlink_color = ACCENT;
    v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0f32, ACCENT);
    v.widgets.active.bg_stroke = egui::Stroke::new(1.0f32, ACCENT);
    ctx.set_visuals(v);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(10.0, 6.0);
    ctx.set_style(style);
}

impl eframe::App for EmulatorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Async ROM-picker result.
        if let Some(rx) = &self.rom_rx {
            match rx.try_recv() {
                Ok(Some(path)) => {
                    self.rom_rx = None;
                    self.load_rom_path(&path);
                }
                Ok(None) | Err(std::sync::mpsc::TryRecvError::Disconnected) => self.rom_rx = None,
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }

        // Drag & drop ROM loading.
        let dropped: Vec<_> = ctx.input(|i| i.raw.dropped_files.clone());
        for file in dropped {
            if let Some(path) = file.path {
                self.load_rom_path(&path);
            } else if let Some(bytes) = file.bytes {
                self.load_rom_bytes(bytes.to_vec(), None);
            }
        }

        // Poll the gamepad once (also records a captured press for rebinding).
        let pad = self.gamepad.poll(&self.controls);

        // Menu bar.
        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open ROM…").clicked() {
                        self.open_rom_dialog();
                        ui.close();
                    }
                    ui.separator();
                    if ui
                        .add_enabled(self.state_path.is_some(), egui::Button::new("Save state (F5)"))
                        .clicked()
                    {
                        self.state_save_requested = true;
                        ui.close();
                    }
                    if ui
                        .add_enabled(self.state_path.is_some(), egui::Button::new("Load state (F9)"))
                        .clicked()
                    {
                        self.state_load_requested = true;
                        ui.close();
                    }
                });
                ui.menu_button("View", |ui| {
                    ui.checkbox(&mut self.show_sidebar, "Sidebar");
                });
                if ui.button("Settings").clicked() {
                    self.settings_open = true;
                }
            });
        });

        // Input (suppressed while the settings modal or a rebind is active).
        if !self.settings_open && self.rebind.is_none() {
            self.gameplay_input(ctx, pad);
        }
        self.run_emulation();
        self.autosave_tick();

        // Panels.
        self.status_bar(ctx);
        if self.show_sidebar {
            self.sidebar(ctx);
        }
        self.settings_modal(ctx);
        self.capture_rebind(ctx);

        egui::CentralPanel::default()
            .frame(
                egui::Frame::NONE.fill(egui::Color32::from_rgb(0x0a, 0x0c, 0x12)),
            )
            .show(ctx, |ui| {
                self.screen_ui(ui);
            });

        // Deferred actions (borrow the whole app).
        if std::mem::take(&mut self.save_requested) {
            self.save_now();
        }
        if std::mem::take(&mut self.state_save_requested) {
            self.save_state_file();
        }
        if std::mem::take(&mut self.state_load_requested) {
            self.load_state_file();
        }

        ctx.request_repaint();
    }

    fn on_exit(&mut self) {
        self.flush_save();
    }
}
