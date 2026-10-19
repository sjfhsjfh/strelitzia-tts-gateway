fn main() {
    println!("cargo:rerun-if-changed=../../assets/icon.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let icon = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/icon.ico");
    let icon = icon.to_str().expect("icon path is valid UTF-8");
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon(icon);
    resource
        .compile()
        .expect("failed to embed the Windows executable icon");
}
