//! Compress redistribution notices at build time, keeping each executable standalone.
pub fn embed() {
    let root =
        std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let mut text = String::new();
    for name in [
        "LICENSE",
        "LICENSE-MIT",
        "LICENSE-APACHE",
        "NOTICE",
        "THIRD_PARTY_NOTICES.md",
        "vendor/cpal/LICENSE",
        "vendor/cpal/LANMIX_PATCH.md",
        "THIRD_PARTY_LICENSES.txt",
    ] {
        let path = root.join(name);
        println!("cargo:rerun-if-changed={}", path.display());
        text.push_str(&format!("\n================ {name} ================\n\n"));
        text.push_str(
            &std::fs::read_to_string(path)
                .expect("redistribution notice")
                .replace("\r\n", "\n"),
        );
        text.push('\n');
    }
    let compressed = miniz_oxide::deflate::compress_to_vec(text.as_bytes(), 9);
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap())
        .join("desktop-notices.deflate");
    std::fs::write(output, compressed).expect("embed redistribution notices");
}
