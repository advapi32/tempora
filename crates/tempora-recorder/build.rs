use std::env;

fn main() -> std::io::Result<()> {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return Ok(());
    }

    // The mark with a record button, so the recorder reads apart from the dashboard.
    let app = "../../assets/icons/recorder.ico";
    let manifest = "../../assets/windows.manifest";
    // The tray icons, by the resource IDs the recorder loads them with.
    let tray = [
        ("../../assets/icons/tray-recording.ico", "101"),
        ("../../assets/icons/tray-paused.ico", "102"),
        ("../../assets/icons/tray-attention.ico", "103"),
    ];
    for path in [app, manifest]
        .into_iter()
        .chain(tray.iter().map(|(path, _)| *path))
    {
        println!("cargo:rerun-if-changed={path}");
    }
    println!("cargo:rerun-if-env-changed=RC_PATH");

    // winresource sets both the numeric and displayed versions from CARGO_PKG_VERSION.
    let mut resource = winresource::WindowsResource::new();
    resource
        .set("ProductName", "Tempora")
        .set("FileDescription", "Tempora recorder")
        .set("InternalName", "tempora-recorder")
        .set("OriginalFilename", "tempora-recorder.exe")
        .set_icon(app)
        .set_manifest_file(manifest);
    for (path, id) in tray {
        resource.set_icon_with_id(path, id);
    }
    resource.compile()
}
