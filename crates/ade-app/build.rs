use std::fs::File;
use std::path::PathBuf;

use ico::{IconDir, IconDirEntry, IconImage, ResourceType};
use image::imageops::FilterType;

const ICON_SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];

fn main() {
    println!("cargo:rerun-if-changed=assets/app-icon.png");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let source = image::open("assets/app-icon.png")
        .expect("failed to decode application icon")
        .into_rgba8();
    let output = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR is not set"))
        .join("app-icon.ico");
    let mut directory = IconDir::new(ResourceType::Icon);
    for size in ICON_SIZES {
        let rgba = image::imageops::resize(&source, size, size, FilterType::Lanczos3).into_raw();
        let image = IconImage::from_rgba_data(size, size, rgba);
        directory.add_entry(IconDirEntry::encode_as_png(&image).expect("failed to encode icon"));
    }
    directory
        .write(File::create(&output).expect("failed to create generated icon"))
        .expect("failed to write generated icon");

    winresource::WindowsResource::new()
        .set_icon(output.to_str().expect("icon path is not valid UTF-8"))
        .compile()
        .expect("failed to embed Windows executable resources");
}
