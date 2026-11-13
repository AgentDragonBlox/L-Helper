use embed_manifest::embed_manifest_file;
use std::path::{Path, PathBuf};

fn main() {
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    // Test build (feature "test-identity", src/identity.rs): its own package name and certificate
    let test = std::env::var_os("CARGO_FEATURE_TEST_IDENTITY").is_some();

    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        // Admin (Lenovo WMI / EnergyDrv), per-monitor DPI, and the sparse package identity link (packaging/)
        let mut manifest = std::fs::read_to_string("lhelper.exe.manifest").expect("lhelper.exe.manifest");
        if test {
            manifest = manifest.replace(r#"publisher="CN=LHelper" packageName="LHelper""#, r#"publisher="CN=LHelperTest" packageName="LHelperTest""#);
            assert!(manifest.contains("LHelperTest"), "lhelper.exe.manifest: package identity not found");
        }
        let manifest_path = out.join("lhelper.exe.manifest");
        std::fs::write(&manifest_path, manifest).unwrap();
        embed_manifest_file(&manifest_path).expect("unable to embed manifest");
        // Exe icon (the blue L-Helper logo) and version. The version matters to the MSI: Windows Installer keeps an
        // existing unversioned exe it thinks was modified (seen with a fork's exe in the same folder), but always
        // replaces it with a versioned one.
        let rc = out.join("lhelper.rc");
        std::fs::write(&rc, resources(test)).unwrap();
        embed_resource::compile(&rc, embed_resource::NONE).manifest_optional().expect("unable to embed icon and version");
    }

    // The sparse package and its certificate, built into the exe (win/package.rs) when packaging\pack.ps1 has
    // made them; empty otherwise, and the app then looks for them next to the exe only
    let bundle = Path::new("packaging").join(if test { "build-test" } else { "build" });
    for name in ["LHelper.msix", "LHelper.cer"] {
        let source = bundle.join(name);
        let bytes = std::fs::read(&source).unwrap_or_default();
        std::fs::write(out.join(name), bytes).unwrap();
        println!("cargo:rerun-if-changed={}", source.display());
    }

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=lhelper.exe.manifest");
    println!("cargo:rerun-if-changed=assets/lhelper.ico");
}

/// Icon and VERSIONINFO (Cargo.toml version), for Explorer's Details tab and Windows Installer's file versioning
fn resources(test: bool) -> String {
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let parts: Vec<u16> = version.split('.').map(|p| p.parse().unwrap_or(0)).collect();
    let (major, minor, patch) = (parts.first().copied().unwrap_or(0), parts.get(1).copied().unwrap_or(0), parts.get(2).copied().unwrap_or(0));
    let icon = Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("assets").join("lhelper.ico").to_string_lossy().replace('\\', "/");
    let (name, file) = if test { ("L-Helper Test", "lhelper-test.exe") } else { ("L-Helper", "lhelper.exe") };
    format!(
        r#"1 ICON "{icon}"
1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "L-Helper"
      VALUE "FileDescription", "{name}"
      VALUE "FileVersion", "{major}.{minor}.{patch}.0"
      VALUE "InternalName", "lhelper"
      VALUE "LegalCopyright", "GPL-3.0-or-later"
      VALUE "OriginalFilename", "{file}"
      VALUE "ProductName", "{name}"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
    )
}
