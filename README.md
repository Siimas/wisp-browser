# Wisp

A fast, lightweight, customizable web browser written in Rust.

Wisp is a thin Rust shell around the web engine your operating system already
ships (WebKit on macOS, WebView2 on Windows). It does not bundle its own copy of
Chromium, so the download is a few megabytes and pages render with the system
engine.

## Features

- Tabs, with links that want a new window opening as tabs instead
- Address bar that takes an address or a search
- Ad and tracker blocking inside the engine's network layer
- In-app settings (`Cmd/Ctrl + ,`), applied live:
  - **Appearance**: theme, accent color, compact toolbar, corner roundness, text size, custom CSS
  - **Behavior**: homepage, new-tab page, search engine, session restore, default zoom
  - **Ad blocking**: on/off and your own extra hosts
  - **Hotkeys**: every shortcut can be rebound
- Session restore, saved as you browse

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
| Next / previous tab | `Ctrl+Tab` / `Ctrl+Shift+Tab` |
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
- No bookmarks, history view, or find-in-page yet.

## License

[MIT](LICENSE)
