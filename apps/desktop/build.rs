fn main() {
    println!("cargo:rerun-if-changed=assets/branding/nemuribeat.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/branding/nemuribeat.ico")
            .set("ProductName", "NemuriBeat")
            .set("FileDescription", "NemuriBeat")
            .compile()
            .expect("embed NemuriBeat Windows icon");
    }
}
