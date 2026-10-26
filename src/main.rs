mod app;
mod crypto;
mod fontmap;
mod game;
mod lpk;
mod settings;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 820.0])
            .with_min_inner_size([1020.0, 640.0])
            .with_title("LOA Font Studio"),
        ..Default::default()
    };

    eframe::run_native(
        "LOA Font Studio",
        options,
        Box::new(|cc| Ok(Box::new(app::FontStudioApp::new(cc)))),
    )
}
