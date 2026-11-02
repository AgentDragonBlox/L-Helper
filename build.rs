use embed_manifest::embed_manifest_file;

fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        // Admin (Lenovo WMI / EnergyDrv), per-monitor DPI, and the sparse package identity link (packaging/)
        embed_manifest_file("lhelper.exe.manifest").expect("unable to embed manifest");
        // Exe icon: the blue L-Helper logo
        embed_resource::compile("lhelper.rc", embed_resource::NONE).manifest_optional().expect("unable to embed icon");
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=lhelper.exe.manifest");
    println!("cargo:rerun-if-changed=lhelper.rc");
    println!("cargo:rerun-if-changed=assets/lhelper.ico");
}
