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

## Default hotkeys

`Mod` is `Cmd` on macOS and `Ctrl` on Windows.

| Action | Keys |
| --- | --- |
| New tab / close tab | `Mod+T` / `Mod+W` |
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

`main` is protected: open a pull request, and CI (`cargo fmt --check`,
`cargo clippy`, `cargo test` on macOS and Windows) must pass before merging.

To cut a release, bump `version` in `Cargo.toml`, merge, then push a matching
tag (`git tag v0.2.0 && git push origin v0.2.0`). The Release workflow builds
the installers and publishes them.

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

## License

[MIT](LICENSE)
