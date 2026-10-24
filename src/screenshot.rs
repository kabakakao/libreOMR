//! Native framebuffer captures for both the wgpu and OpenGL backends.
use eframe::egui;

#[derive(Clone, Default)]
struct CaptureState {
    frames: u32,
    requested: bool,
}

pub fn capture(context: &egui::Context) {
    let Some(path) = std::env::var_os("LIBREOMR_SCREENSHOT_TO") else {
        return;
    };
    let image = context.input(|input| {
        input.events.iter().find_map(|event| {
            if let egui::Event::Screenshot {
                viewport_id, image, ..
            } = event
                && *viewport_id == egui::ViewportId::ROOT
            {
                Some(image.clone())
            } else {
                None
            }
        })
    });
    if let Some(image) = image {
        let pixels: Vec<u8> = image
            .pixels
            .iter()
            .flat_map(|pixel| pixel.to_array())
            .collect();
        image::save_buffer(
            std::path::Path::new(&path),
            &pixels,
            image.width() as u32,
            image.height() as u32,
            image::ColorType::Rgba8,
        )
        .expect("write native screenshot");
        context.send_viewport_cmd(egui::ViewportCommand::Close);
        return;
    }
    let request = context.data_mut(|data| {
        let state =
            data.get_temp_mut_or_default::<CaptureState>(egui::Id::new("native_screenshot"));
        state.frames += 1;
        if state.frames >= 4 && !state.requested {
            state.requested = true;
            true
        } else {
            false
        }
    });
    if request {
        context.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
    }
    context.request_repaint();
}
