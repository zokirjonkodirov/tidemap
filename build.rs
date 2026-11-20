//! Windows: embeds the Tidemap mark and version details in Tidemap.exe, so Explorer, the taskbar
//! and Task Manager show the icon and the name before the app has even started. Other targets
//! need nothing: macOS and Linux take the icon from `assets/icon.png` at run time and from the
//! launcher files.
//!
//! A machine without a resource compiler (rc.exe for MSVC, windres for GNU) still builds the app;
//! the executable just has no icon, and Cargo prints a warning saying so.

use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("set by Cargo"));
    let icon = root.join("assets").join("icon.ico");
    let version = env::var("CARGO_PKG_VERSION").expect("set by Cargo");
    let mut parts: Vec<u16> = version
        .split(['.', '-', '+'])
        .take(3)
        .map(|part| part.parse().unwrap_or(0))
        .collect();
    parts.resize(4, 0);
    let numeric = parts
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let icon_path = icon.display().to_string().replace('\\', "\\\\");

    let script = format!(
        r#"1 ICON "{icon_path}"

1 VERSIONINFO
FILEVERSION {numeric}
PRODUCTVERSION {numeric}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "FileDescription", "Tidemap"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "tidemap"
      VALUE "OriginalFilename", "Tidemap.exe"
      VALUE "ProductName", "Tidemap"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
    );
    let resource = PathBuf::from(env::var("OUT_DIR").expect("set by Cargo")).join("tidemap.rc");
    fs::write(&resource, script).expect("OUT_DIR is writable");
    if let Err(problem) =
        embed_resource::compile(&resource, embed_resource::NONE).manifest_optional()
    {
        println!("cargo:warning=Tidemap.exe is built without its icon: {problem}");
    }
}
