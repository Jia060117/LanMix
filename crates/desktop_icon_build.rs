//! Embed the shared icon and publisher metadata without adding a build dependency.
pub fn embed() {
    println!("cargo:rerun-if-changed=../desktop_icon_build.rs");
    println!("cargo:rerun-if-changed=../../assets/branding/lanmix.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
        || std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc")
    {
        return;
    }
    use std::path::PathBuf;
    let icon = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("../../assets/branding/lanmix.ico")
        .canonicalize()
        .expect("shared Windows icon");
    let icon = icon
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .replace('\\', "\\\\");
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let numeric = version.split('.').collect::<Vec<_>>().join(",");
    let package = std::env::var("CARGO_PKG_NAME").unwrap();
    let role = if package.ends_with("sender") {
        "Sender"
    } else {
        "Receiver"
    };
    let output = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let source = output.join("lanmix.rc");
    let resource = output.join("lanmix.res");
    std::fs::write(
        &source,
        format!(
            r#"
1 ICON "{icon}"
1 VERSIONINFO
FILEVERSION {numeric},0
PRODUCTVERSION {numeric},0
FILEFLAGSMASK 0x3fL
FILEFLAGS 0
FILEOS 0x40004L
FILETYPE 0x1L
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "CompanyName", "Jia060117\0"
      VALUE "FileDescription", "LanMix {role}\0"
      VALUE "FileVersion", "{version}\0"
      VALUE "InternalName", "{package}\0"
      VALUE "LegalCopyright", "Copyright (c) 2026 Jia060117\0"
      VALUE "OriginalFilename", "{package}.exe\0"
      VALUE "ProductName", "LanMix\0"
      VALUE "ProductVersion", "{version}\0"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x0409, 1200
  END
END
"#
        ),
    )
    .expect("write Windows resources");
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|p| p.join("rc.exe")));
    }
    for key in ["WindowsSdkVerBinPath", "WindowsSdkBinPath"] {
        println!("cargo:rerun-if-env-changed={key}");
        if let Some(path) = std::env::var_os(key) {
            candidates.push(PathBuf::from(&path).join("x64/rc.exe"));
            candidates.push(PathBuf::from(path).join("rc.exe"));
        }
    }
    if let Some(programs) = std::env::var_os("ProgramFiles(x86)") {
        let bin = PathBuf::from(programs).join("Windows Kits/10/bin");
        let mut versions: Vec<_> = std::fs::read_dir(bin)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.join("x64/rc.exe").is_file())
            .collect();
        versions.sort_by_key(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .split('.')
                .map(|part| part.parse::<u32>().unwrap_or(0))
                .collect::<Vec<_>>()
        });
        candidates.extend(
            versions
                .into_iter()
                .rev()
                .map(|path| path.join("x64/rc.exe")),
        );
    }
    let compiler = candidates
        .into_iter()
        .find(|path| path.is_file())
        .expect("Windows SDK rc.exe is required for executable icons");
    let status = std::process::Command::new(compiler)
        .arg("/nologo")
        .arg("/fo")
        .arg(&resource)
        .arg(&source)
        .status()
        .expect("run Windows resource compiler");
    assert!(status.success(), "Windows resource compilation failed");
    println!("cargo:rustc-link-arg-bins={}", resource.display());
}
