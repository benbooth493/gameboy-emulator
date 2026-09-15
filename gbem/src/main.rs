mod app;
mod audio;
mod gamepad;
mod pacing;
mod saves;
mod screen;

fn main() -> eframe::Result<()> {
    env_logger::init();
    let rom_path = std::env::args().nth(1);

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 720.0])
            .with_title("gbem — Game Boy emulator"),
        ..Default::default()
    };
    eframe::run_native(
        "gbem",
        options,
        Box::new(move |cc| Ok(Box::new(app::EmulatorApp::new(cc, rom_path)))),
    )
}
