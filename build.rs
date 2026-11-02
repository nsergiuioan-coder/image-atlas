use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=assets/app.ico");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let icon = root
        .join("assets/app.ico")
        .to_string_lossy()
        .replace('\\', "/");
    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let major = env::var("CARGO_PKG_VERSION_MAJOR").unwrap();
    let minor = env::var("CARGO_PKG_VERSION_MINOR").unwrap();
    let patch = env::var("CARGO_PKG_VERSION_PATCH").unwrap();
    let resource = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("atlas.rc");
    fs::write(
        &resource,
        format!(
            r#"#pragma code_page(65001)
1 ICON "{icon}"
1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904B0"
        BEGIN
            VALUE "CompanyName", "NIS\0"
            VALUE "FileDescription", "Image Atlas\0"
            VALUE "FileVersion", "{version}\0"
            VALUE "InternalName", "ImageAtlas\0"
            VALUE "LegalCopyright", "Copyright (c) 2026 NIS\0"
            VALUE "OriginalFilename", "ImageAtlas.exe\0"
            VALUE "ProductName", "Image Atlas\0"
            VALUE "ProductVersion", "{version}\0"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x409, 1200
    END
END
"#
        ),
    )
    .expect("Could not write Windows resources");
    embed_resource::compile(resource, embed_resource::NONE)
        .manifest_required()
        .expect("Could not embed the Atlas icon and version");
}
