use std::env;

fn main() -> std::io::Result<()> {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return Ok(());
    }

    let app = "../../assets/icons/app.ico";
    let manifest = "../../assets/windows.manifest";
    for path in [app, manifest] {
        println!("cargo:rerun-if-changed={path}");
    }
    println!("cargo:rerun-if-env-changed=RC_PATH");

    // winresource sets both the numeric and displayed versions from CARGO_PKG_VERSION.
    winresource::WindowsResource::new()
        .set("ProductName", "Tempora")
        .set("FileDescription", "Tempora")
        .set("InternalName", "tempora")
        .set("OriginalFilename", "tempora.exe")
        .set_icon(app)
        .set_manifest_file(manifest)
        .compile()
}
