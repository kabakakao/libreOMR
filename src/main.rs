mod app;
mod icon;
mod preview;
#[cfg(feature = "ui-screenshot")]
mod screenshot;
mod settings;
mod startup;
mod theme;

fn main() -> std::process::ExitCode {
    settings::Settings::load().apply_language();
    startup::install_panic_hook();
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            startup::report_error(&format!("{error}\n\n{error:?}"));
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> eframe::Result {
    let compact_capture = cfg!(feature = "ui-screenshot")
        && std::env::var_os("LIBREOMR_COMPACT_SCREENSHOT").is_some();
    let options = eframe::NativeOptions {
        // Keep OpenGL on Linux/macOS; Windows uses the bundled wgpu renderer.
        #[cfg(target_os = "windows")]
        renderer: eframe::Renderer::Wgpu,
        #[cfg(not(target_os = "windows"))]
        renderer: eframe::Renderer::Glow,
        #[cfg(target_os = "windows")]
        wgpu_options: startup::windows_wgpu_options(),
        viewport: eframe::egui::ViewportBuilder::default()
            .with_app_id("org.libreomr.libreomr")
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
