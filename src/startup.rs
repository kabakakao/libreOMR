//! Error reporting that works even before the graphics renderer is available.

use std::io::Write;
use std::path::{Path, PathBuf};

pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        previous(info);
        let details = format!("{info}\n\n{}", std::backtrace::Backtrace::force_capture());
        report(&info.to_string(), &details);
    }));
}

pub fn report_error(error: &str) {
    report(error, error);
}

fn report(error: &str, details: &str) {
    let report = diagnostic_report(details);
    let log_message = match save_report(&report) {
        Ok(path) => format!("Fehlerbericht: {}", path.display()),
        Err(failure) => format!("Der Fehlerbericht konnte nicht gespeichert werden: {failure}"),
    };
    let message = format!(
        "libreOMR wurde wegen eines Fehlers beendet.\n\n{}\n\n{log_message}\n\nTechnische Details:\n{error}",
        recovery_help()
    );
    // Preserve diagnostics when launched from a console, including log failures.
    // Ignore a closed stderr: reporting must not itself panic.
    let _ = writeln!(std::io::stderr(), "{message}\n\n{report}");
    rfd::MessageDialog::new()
        .set_title("libreOMR - Fehler")
        .set_description(&message)
        .set_level(rfd::MessageLevel::Error)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

fn diagnostic_report(details: &str) -> String {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    format!(
        "libreOMR {} | {} {} | Unix time: {timestamp}\n{}\n\n{details}\n\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        recovery_help(),
    )
}

fn save_report(report: &str) -> std::io::Result<PathBuf> {
    #[cfg(target_os = "windows")]
    if let Some(directory) = std::env::var_os("LOCALAPPDATA").filter(|value| !value.is_empty())
        && let Ok(path) = write_report_to(&PathBuf::from(directory).join("libreOMR"), report)
    {
        return Ok(path);
    }
    write_report_to(&std::env::temp_dir().join("libreOMR"), report)
}

fn write_report_to(directory: &Path, report: &str) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(directory)?;
    let path = directory.join("error.log");
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    file.write_all(report.as_bytes())?;
    Ok(path)
}

fn recovery_help() -> &'static str {
    if cfg!(target_os = "windows") {
        "Bei Grafikfehlern: Windows und den Grafiktreiber vom Gerätehersteller aktualisieren. \
         In einer VM auch die Grafikunterstützung der Gast-Erweiterungen prüfen.\n\n\
         Dieser Build enthält den wgpu-Renderer und verwendet Direct3D 12 statt OpenGL. \
         Als Alternative kann Vulkan versucht werden (PowerShell):\n\
         $env:WGPU_BACKEND = 'vulkan'; .\\libreomr.exe\n\n\
         Bitte bei einer Fehlermeldung den Fehlerbericht beifügen."
    } else {
        "Bei Grafikfehlern den Grafiktreiber und die OpenGL-Unterstützung prüfen. \
         Bitte bei einer Fehlermeldung den Fehlerbericht beifügen."
    }
}

#[cfg(target_os = "windows")]
pub fn windows_wgpu_options() -> eframe::egui_wgpu::WgpuConfiguration {
    use eframe::egui_wgpu::{WgpuSetupCreateNew, wgpu};

    let mut setup = WgpuSetupCreateNew::default();
    // Prefer Windows' system graphics API. Allow an explicit Vulkan override for
    // machines with a broken D3D driver without loading OpenGL during normal startup.
    setup.instance_descriptor.backends = wgpu::Backends::from_env().unwrap_or(wgpu::Backends::DX12);
    // FXC is part of Windows; do not require a separately installed DXC DLL.
    setup.instance_descriptor.backend_options.dx12.shader_compiler = wgpu::Dx12Compiler::Fxc;
    eframe::egui_wgpu::WgpuConfiguration {
        wgpu_setup: setup.into(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_preserve_previous_failures_and_unicode_details() {
        let directory = tempfile::tempdir().unwrap();
        let log_directory = directory.path().join("new-directory");
        let first = diagnostic_report("OpenGL(PainterError(\"egui_glow requires opengl 2.0+.\"))");
        let second = diagnostic_report("Grafikgerät nicht verfügbar");
        let path = write_report_to(&log_directory, &first).unwrap();
        assert_eq!(write_report_to(&log_directory, &second).unwrap(), path);
        let saved = std::fs::read_to_string(path).unwrap();
        assert_eq!(saved, format!("{first}{second}"));
        assert!(saved.contains(env!("CARGO_PKG_VERSION")));
        assert!(saved.contains(std::env::consts::ARCH));
    }

    #[test]
    fn unwritable_log_directory_is_reported_as_an_error() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("file");
        std::fs::write(&file, "existing file").unwrap();
        assert!(write_report_to(&file, "failure").is_err());
        assert_eq!(std::fs::read_to_string(file).unwrap(), "existing file");
    }
}
