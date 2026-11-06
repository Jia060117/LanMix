//! Notices are read only on explicit export, never on the audio path or at startup.
pub fn export(path: &std::path::Path) -> anyhow::Result<()> {
    let bytes = miniz_oxide::inflate::decompress_to_vec_with_limit(
        include_bytes!(concat!(env!("OUT_DIR"), "/desktop-notices.deflate")),
        16 * 1024 * 1024,
    )
    .map_err(|error| anyhow::anyhow!("decode embedded notices: {error:?}"))?;
    std::fs::write(path, bytes)?;
    Ok(())
}
