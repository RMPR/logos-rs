fn main() {
    slint_build::compile("ui/app.slint").expect("Slint build failed");
    // The executable's own icon, as Explorer and the taskbar show it.
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("packaging/windows/leverbe.ico");
        res.compile().expect("Windows resources");
    }
}
