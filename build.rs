//! Windows: give wisp.exe the identity a well-behaved program has. Without a version
//! resource, an icon and a manifest an unsigned executable is anonymous to the system
//! (Explorer's Details tab is blank, Task Manager shows "wisp.exe"), and antivirus
//! heuristics score anonymous unsigned programs worse.

fn main() {
    // Only when building on Windows for Windows: the resource compiler (rc.exe) comes
    // with the Windows SDK, and a cross-target `cargo clippy` from macOS has none.
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=assets/icon.ico");
        println!("cargo:rerun-if-changed=assets/wisp.exe.manifest");
        winresource::WindowsResource::new()
            .set_icon("assets/icon.ico")
            .set_manifest_file("assets/wisp.exe.manifest")
            // Windows shows FileDescription as the program's name (Task Manager, "Open with").
            .set("FileDescription", "Wisp")
            .set("ProductName", "Wisp")
            .set("CompanyName", "Siimas")
            .set("LegalCopyright", "Copyright (c) 2026 Siimas")
            .set("OriginalFilename", "wisp.exe")
            .set("InternalName", "wisp")
            .compile()
            .expect("could not embed the Windows resources (is the Windows SDK installed?)");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
