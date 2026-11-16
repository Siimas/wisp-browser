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
        self.update.install(self.bytes).map_err(|e| e.to_string())
    }
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
    // The address may be overridden (for testing against a local server); what is
    // downloaded must still carry a signature made with the release key.
    let endpoint = std::env::var("WISP_UPDATE_ENDPOINT").unwrap_or_else(|_| ENDPOINT.into());
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
