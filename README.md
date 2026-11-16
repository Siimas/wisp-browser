# Wisp

A fast, lightweight, customizable web browser written in Rust.

Wisp is a thin Rust shell around the web engine your operating system already
ships (WebKit on macOS, WebView2 on Windows). It does not bundle its own copy of
Chromium, so the download is a few megabytes and pages render with the system
engine.

## Features

- **Sidebar or top bar**: tabs in a sidebar on the right (default) or left with
  a hotbar across the top for the address bar and buttons, or a classic top bar
  in relaxed or compact size. The page sits on a rounded card.
- **Customize toolbar**: drag the address bar, any button, the workspace
  switcher and spacers between the hotbar and the sidebar, or remove them
- **Collapsible bar**: tuck the bar away; it floats back over the page when the
  mouse comes near that window edge
- **Workspaces**: named groups of tabs with their own icon and color; the
  window takes a tint of the active one, and tabs can be moved between them
- **Tab basics**: drag tabs to reorder them, pin the ones you keep (compact
  icon tiles above the list), duplicate a tab, close the others, and reopen
  the tab you just closed
- **Private tabs and private windows**: nothing from them is kept on disk or
  saved in the session
- **Theme editor**: presets, up to three background colors as a gradient,
  textures (grain, dots, lines, grid), accent color, custom CSS
- Ad and tracker blocking inside the engine's network layer
- Every hotkey can be rebound
- Session restore, saved as you browse; restored tabs load only when opened
- Double-click an empty part of the bar to expand the window, drag it to move,
  right-click it for layout options

Everything is in Settings (`Cmd/Ctrl + ,`) and applies live. The interface is
built on a small design and motion system: see [docs/DESIGN.md](docs/DESIGN.md).

## Install

Download the installer for your system from the
[latest release](https://github.com/Siimas/wisp-browser/releases/latest).

| System | File |
| --- | --- |
| macOS, Apple Silicon | `Wisp_<version>_aarch64.dmg` |
| macOS, Intel | `Wisp_<version>_x64.dmg` |
| Windows 10/11, 64-bit | `Wisp_<version>_x64-setup.exe` |

The installers are not code-signed yet, so the first launch needs one extra step:

- **macOS**: after dragging Wisp to Applications, run
  `xattr -cr /Applications/Wisp.app` in Terminal, then open it.
- **Windows**: on the SmartScreen prompt choose **More info**, then **Run anyway**.

## Updates

Installed copies update themselves. A new version downloads in the background,
is installed as you quit, and is simply there the next time you open Wisp, with
its release notes in a tab. You can turn this off, or check by hand, in
Settings > About.

Every update file is signed, and Wisp refuses any file whose signature does not
match the key it was built with. Copies you build yourself do not update.

## Uninstall

**macOS**: quit Wisp and drag `Wisp.app` from Applications to the Trash. That
removes the app and leaves your data. To remove the data as well:

```bash
rm -rf ~/Library/"Application Support"/Wisp ~/Library/WebKit/io.github.siimas.wisp ~/Library/Caches/io.github.siimas.wisp ~/Library/HTTPStorages/io.github.siimas.wisp*
```

**Windows**: open Settings > Apps > Installed apps, find Wisp and choose
**Uninstall** (or run `uninstall.exe` in the install folder). This removes the
program, its Start menu and desktop shortcuts and its entry in the list. Tick
**Delete the application data** in the uninstaller to remove your data too, or
delete the folders below by hand.

What lives where:

| | macOS | Windows |
| --- | --- | --- |
| Settings and saved session | `~/Library/Application Support/Wisp/` | `%APPDATA%\Wisp\` |
| Cookies, cache, site storage | `~/Library/WebKit/io.github.siimas.wisp/`, `~/Library/Caches/io.github.siimas.wisp/`, `~/Library/HTTPStorages/io.github.siimas.wisp*` | `wisp.exe.WebView2\` in the install folder (`%LOCALAPPDATA%\Wisp\` unless you chose another) |

Wisp keeps nothing anywhere else. See [PRIVACY.md](PRIVACY.md).

## Default hotkeys

`Mod` is `Cmd` on macOS and `Ctrl` on Windows.

| Action | Keys |
| --- | --- |
| New tab / close tab | `Mod+T` / `Mod+W` |
| Reopen closed tab | `Mod+Shift+T` |
| New private tab / private window | `Mod+Shift+P` / `Mod+Shift+N` |
| Next / previous tab | `Ctrl+Tab` / `Ctrl+Shift+Tab` |
| Next / previous workspace | `Mod+Alt+Right` / `Mod+Alt+Left` |
| Collapse or show the bar | `Mod+Shift+B` |
| Focus address bar | `Mod+L` |
| Back / forward | `Mod+[` / `Mod+]` |
| Reload / home | `Mod+R` / `Mod+Shift+H` |
| Zoom in / out / reset | `Mod+=` / `Mod+-` / `Mod+0` |
| Settings | `Mod+,` |

## Settings file

Settings live in `config.json`, which the settings page writes for you:

- macOS: `~/Library/Application Support/Wisp/`
- Windows: `%APPDATA%\Wisp\`

## Build from source

Needs [Rust](https://rustup.rs). On Windows you also need the
[WebView2 runtime](https://developer.microsoft.com/microsoft-edge/webview2/)
(preinstalled on Windows 11).

```bash
cargo run --release
```

To build an installer:

```bash
cargo install cargo-packager --locked
cargo build --release
cargo packager --release --formats dmg   # or: --formats nsis on Windows
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for how to build, check and submit a
change, and [CHANGELOG.md](CHANGELOG.md) for what changed in each version.
`main` is protected: open a pull request, and CI (`cargo fmt --check`,
`cargo clippy`, `cargo test` on macOS and Windows) must pass before merging.

To cut a release, move the Unreleased entries in `CHANGELOG.md` under the new
version, bump `version` in `Cargo.toml`, merge, then push a matching
tag (`git tag v0.2.0 && git push origin v0.2.0`). The Release workflow builds
the installers and publishes them.

### Update signing key (one-time setup)

Automatic updates need a signing key pair. Generate it once and keep the
private half safe: anyone holding it can ship an update to every installed copy,
and if it is lost, installed copies can no longer be updated automatically.

```bash
cargo packager signer generate --path ~/.wisp/update.key
```

Then, in the repository's settings on GitHub (Secrets and variables > Actions):

- secret `WISP_UPDATE_PRIVATE_KEY`: the contents of `~/.wisp/update.key`
- secret `WISP_UPDATE_PRIVATE_KEY_PASSWORD`: the password you chose
- variable `WISP_UPDATE_PUBKEY`: the contents of `~/.wisp/update.key.pub`

From the next release on, the workflow signs the update files and publishes
`latest.json`, which installed copies read. Without the key, releases are built
as before and offer no automatic update.

## Known limits

- Ad blocking is a host list (built-in plus your own). It does not yet read
  EasyList-style filter lists or hide leftover empty ad slots.
- Linux builds are not provided, and network-level blocking is not wired up there.
- Workspaces group tabs but share cookies and logins.
- A menu that has to overlap the page hides the page while it is open (menus
  opened from the sidebar stay inside it). Page corners are square on Windows.
- A private window runs as a second copy of the app, so it shows its own
  Dock or taskbar icon.
- No bookmarks, history view, or find-in-page yet.

## Privacy

No telemetry, no accounts, no Wisp server. The app itself only contacts GitHub
to check for updates and the sites in your tabs for their icons. The full
statement is in [PRIVACY.md](PRIVACY.md).

## License

[MIT](LICENSE). Wisp is built from open-source Rust crates; their licenses are
in [THIRD-PARTY-NOTICES](THIRD-PARTY-NOTICES), also reachable from Settings >
About.
