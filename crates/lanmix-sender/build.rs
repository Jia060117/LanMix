#[path = "../desktop_icon_build.rs"]
mod icon;
#[path = "../desktop_notices_build.rs"]
mod notices;

fn main() {
    notices::embed();
    icon::embed();
    println!("cargo:rerun-if-changed=src/windows.manifest");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        let manifest = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("src/windows.manifest");
        println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}",
            manifest.display()
        );
    }
}
