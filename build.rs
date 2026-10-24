fn main() {
    println!("cargo:rerun-if-changed=assets/icons/libreomr.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var_os("CARGO_FEATURE_DESKTOP").is_some()
    {
        winresource::WindowsResource::new()
            .set_icon("assets/icons/libreomr.ico")
            .set("ProductName", "libreOMR")
            .set("FileDescription", "libreOMR")
            .set("OriginalFilename", "libreomr.exe")
            .compile_for("libreomr")
            .expect("could not embed the Windows icon; check the Windows SDK resource compiler");
    }
}
