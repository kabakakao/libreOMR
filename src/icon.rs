const ICON_PNG: &[u8] = include_bytes!("../assets/libreomr-icon-dark.png");

pub fn window_icon() -> eframe::egui::IconData {
    eframe::icon_data::from_png_bytes(ICON_PNG).expect("embedded app icon must be a valid PNG")
}

// Window icons do not set the macOS Dock icon. Set it through AppKit as well,
// including when the executable is launched directly with `cargo run`.
#[cfg(target_os = "macos")]
pub fn set_dock_icon() {
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let main_thread = MainThreadMarker::new().expect("the app must start on the main thread");
    let data = NSData::with_bytes(ICON_PNG);
    let image = NSImage::initWithData(NSImage::alloc(), &data)
        .expect("embedded app icon must be readable by AppKit");
    let application = NSApplication::sharedApplication(main_thread);
    // SAFETY: Both the application and image are valid and this runs on the
    // main thread after eframe has initialized the native application.
    unsafe { application.setApplicationIconImage(Some(&image)) };
}
