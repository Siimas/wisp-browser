//! Automatic updates from the project's GitHub releases.
//!
//! The release workflow publishes `latest.json` next to the installers. Each update
//! file is signed with a private key that only the release workflow holds; this app is
//! built with the matching public key and refuses anything the signature does not
//! match. A copy built without a key (a local build) never updates itself.

use cargo_packager_updater::{Config, Update, check_update};

/// Public half of the signing key, baked in when the release is built.
const PUBKEY: Option<&str> = option_env!("WISP_UPDATE_PUBKEY");
const ENDPOINT: &str =
    "https://github.com/Siimas/wisp-browser/releases/latest/download/latest.json";

/// A newer version, downloaded and signature-checked, waiting to be installed.
pub struct Ready {
    pub version: String,
    update: Update,
    bytes: Vec<u8>,
}

impl Ready {
    /// Put the new version in place of this one. On macOS the app bundle is swapped and
    /// this returns; on Windows the installer is started and the process exits.
    pub fn install(self) -> Result<(), String> {
        #[cfg(windows)]
        return run_installer(&self.update, &self.bytes);
        #[cfg(not(windows))]
        self.update.install(self.bytes).map_err(|e| e.to_string())
    }
}

/// Windows: start the downloaded installer directly and exit so it can replace us.
///
/// The updater crate would start it through a hidden PowerShell window
/// (`powershell -WindowStyle Hidden Start-Process ...`). A program that downloads an
/// executable and runs it through hidden PowerShell is what malware droppers do, and
/// antivirus behaviour monitoring treats it that way. Starting the installer as a plain
/// child process does the same job without the script host or the hidden window.
/// `bytes` are exactly what `fetch` downloaded and signature-checked.
#[cfg(windows)]
fn run_installer(update: &Update, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    if !matches!(update.format, cargo_packager_updater::UpdateFormat::Nsis) {
        return Err(format!("unexpected update format {}", update.format));
    }
    // Under a name that says what it is, in the user's temp folder (the version is a
    // parsed semver, so it is safe in a file name). A leftover from an earlier attempt
    // is replaced; create_new never writes through a file someone else put there.
    // Closed before it runs, or Windows refuses to start it.
    let path = std::env::temp_dir().join(format!("Wisp-{}-Setup.exe", update.version));
    let _ = std::fs::remove_file(&path);
    let mut file = std::fs::File::create_new(&path).map_err(|e| e.to_string())?;
    file.write_all(bytes).map_err(|e| e.to_string())?;
    drop(file);
    // /P: passive, so the installer's own progress window shows but asks nothing;
    // /R: start Wisp again when done.
    // The installer asks for no administrator rights (it installs for this user only).
    std::process::Command::new(&path)
        .args(["/P", "/R"])
        .spawn()
        .map_err(|e| e.to_string())?;
    std::process::exit(0);
}

/// Whether this copy of the app is able to update itself.
pub fn supported() -> bool {
    let keyed = PUBKEY.is_some_and(|k| !k.trim().is_empty());
    // The updater swaps the installed app; a loose binary (cargo run) has nothing to swap.
    let installed = if cfg!(target_os = "macos") {
        std::env::current_exe().is_ok_and(|p| p.to_string_lossy().contains(".app/Contents/MacOS/"))
    } else {
        cfg!(windows)
    };
    keyed && installed
}

/// Look for a newer release and download it. Blocks on the network: call it off the
/// main thread. `Ok(None)` means this is already the newest version.
pub fn fetch() -> Result<Option<Ready>, String> {
    let fail = |e: &dyn std::fmt::Display| e.to_string();
    let pubkey = PUBKEY
        .filter(|_| supported())
        .ok_or("updates are not set up in this build")?;
    // Development builds may point at a local test server; a released build only ever
    // asks the project's own releases. Either way the file must carry a signature made
    // with the release key, and only a version newer than this one is accepted.
    let endpoint = std::env::var("WISP_UPDATE_ENDPOINT")
        .ok()
        .filter(|_| cfg!(debug_assertions))
        .unwrap_or_else(|| ENDPOINT.into());
    let config = Config {
        endpoints: vec![endpoint.parse().map_err(|e| fail(&e))?],
        pubkey: pubkey.trim().into(),
        ..Default::default()
    };
    let current = env!("CARGO_PKG_VERSION").parse().map_err(|e| fail(&e))?;
    let Some(update) = check_update(current, config).map_err(|e| fail(&e))? else {
        return Ok(None);
    };
    let bytes = update.download().map_err(|e| fail(&e))?; // verifies the signature
    Ok(Some(Ready {
        version: update.version.clone(),
        update,
        bytes,
    }))
}
