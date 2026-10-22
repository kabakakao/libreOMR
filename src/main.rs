mod app;
mod icon;
mod preview;
mod theme;

fn main() -> eframe::Result {
    let compact_capture = cfg!(feature = "ui-screenshot")
        && std::env::var_os("LIBREOMR_COMPACT_SCREENSHOT").is_some();
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_icon(icon::window_icon())
            .with_inner_size(if compact_capture {
                [760.0, 520.0]
            } else {
                [1280.0, 820.0]
            })
            .with_min_inner_size([760.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native(
        "libreOMR",
        options,
        Box::new(|context| {
            #[cfg(target_os = "macos")]
            icon::set_dock_icon();
            Ok(Box::new(app::App::new(context)))
        }),
    )
}
