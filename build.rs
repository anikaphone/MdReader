#[cfg(windows)]
fn main() {
    let mut res = winres::WindowsResource::new();
    res.set_icon("assets/app_icon.ico");
    res.set("ProductName", "MdReader");
    res.set("FileDescription", "Fast Markdown Reader");
    res.set("LegalCopyright", "Copyright (C) 2026 MdReader");
    let _ = res.compile();
}

#[cfg(not(windows))]
fn main() {}
