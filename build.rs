fn main() {
    // On Android Slint would use the Material style, whose ripples and
    // transitions an e-ink screen redraws frame by frame; the desktop's
    // Fluent style is calmer, and keeps the two builds looking alike.
    let mut config = slint_build::CompilerConfiguration::new();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        config = config.with_style("fluent".into());
    }
    slint_build::compile_with_config("ui/app.slint", config).expect("Slint build failed");
    // The executable's own icon, as Explorer and the taskbar show it.
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("packaging/windows/leverbe.ico");
        res.compile().expect("Windows resources");
    }
}
