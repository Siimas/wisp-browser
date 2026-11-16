# Changelog

All notable changes to Wisp are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Split view: show two to four tabs of a workspace side by side, in rows and
  columns. Drag a tab from the list onto any side of the page (the tiles make
  room to show where it lands), choose "Split with current tab" in a tab's menu,
  or press `Cmd/Ctrl+Shift+\`. Drag the gap between tiles to resize, double-click
  it to even them out, and drag a tile's tab onto another tile to rearrange. The
  tile you click in gets the address bar, find and page hotkeys. A split reads
  as one row in the tab list, survives workspace switches and restarts, and
  "Remove from split" or "Unsplit tabs" in a tab's menu take it apart.
- Tab unloading: a background tab not looked at for 30 minutes (Settings >
  Tabs and browsing > Performance: Never to 4 hours) gives its memory back and
  reloads, at the same scroll position, when you return to it. Tabs playing sound, in a call or with
  typed text, private tabs and (unless allowed) pinned tabs stay loaded. An
  unloaded tab's icon rests faded. "Unload tab" and "Unload other tabs" are in
  the tab's right-click menu. Ten busy sites went from 2.3 GB to 0.5 GB with
  nine unloaded.
  An unloaded tab comes back as a fresh load of its current address, so it
  loses its back and forward history, `sessionStorage` and any state held in
  the page, and the position of scrolling areas inside the page (the page's own
  scroll position is restored). A page that was the result of a form
  submission reloads as a plain visit. Typed text is noticed in the page's own
  fields and editors, not inside frames or shadow DOM.
- The settings button in the toolbar opens an app menu: new tabs and windows,
  History, Bookmarks, Downloads, Find in page, zoom, and quick switches for ad
  blocking, light or dark, and collapsing the bar, each with its hotkey.
- Menus now float over the page on glass instead of hiding it while open.
- Low-power mode (Settings > Behavior): solid surfaces instead of glass and no
  animation. Automatic, the default, turns it on in the system's power saving
  mode, on a low battery, or on a computer with 4 GB of memory or less or two
  cores or fewer. The system's "Reduce transparency" setting now turns glass
  off too.
- A media player at the bottom of the sidebar while another tab plays audio or
  video: title and artist, previous, play/pause, next, mute, and a progress line
  you can drag to seek. Click the title to go to the tab; close it to hide it
  until playback starts again. In the top-bar layouts it is a button beside
  "New tab" that opens the same player. A playing tab shows a speaker you can
  click to mute it.

### Changed

- Settings opens as a tab of its own, which you can leave and come back to.
  It is not restored after a restart.
- The interface uses about 50 MB less memory: the background texture is drawn
  without a separate window-sized layer, and looks the same.
- With the bar collapsed, the cursor is checked less often while it is far from
  the bar's edge.
- Google is the default search engine and home page for new profiles. Existing
  profiles keep what they have; change it in Settings > Behavior.
- Double-clicking the empty space around the tabs opens a new tab at the end of
  the list. (Double-clicking the hotbar still expands the window.)
- The sidebar is resized by dragging its inner edge (double-click to reset); the
  width slider is gone from Settings.

## [0.2.1] - 2026-11-16

### Added

- Nine new theme presets (Wisp, Mist, Midnight, Dusk, Aurora, Ember, Daybreak,
  Linen, Glade) with a picker that shows each one's real colors and name.
- A signature look: the window is painted with a soft gradient taken from the
  workspace's color, with a finer grain that also removes banding.

### Changed

- Spacing, sizing and alignment were measured and corrected across the whole
  interface: settings rows, menus, the theme editor, address suggestions,
  history and bookmarks rows, tab close buttons and the macOS window buttons.
- The floating bar's glass takes the theme's own color.
- The workspace switcher scrolls when there are more workspaces than fit.

### Fixed

- Wisp used 10 to 20% of a processor core while idle: a hidden loading bar
  kept animating. It now runs only while a page is loading.
- The find box and the downloads list were narrower than designed, and the
  bookmark button did not show when a page was bookmarked.
- On a floating top bar the macOS window buttons sat above the tab row.

## [0.2.0] - 2026-11-16

### Added

- Find in page (Cmd/Ctrl+F): a find bar above the page, with next and previous
  match. On Windows it also shows how many matches there are.
- Downloads: files are saved to the Downloads folder without replacing existing
  ones, and a toolbar button lists this session's downloads, with Open and Show
  in folder for finished files.
- Right-clicking a link or image in a page offers to open it in a new tab, and
  links in a private tab.
- History: visited pages are remembered on this computer only (never from
  private tabs), listed by day in a History sheet with search, delete, and
  "Clear browsing data" for a time range, optionally with cookies and site
  data. Settings > Behavior > Keep history turns recording off.
- Bookmarks: a toolbar button and a shortcut to bookmark the current page, and
  a Bookmarks sheet to search, rename and delete them.
- Address bar suggestions from open tabs, bookmarks and history, ranked by how
  often and how recently a page was visited. Nothing is sent anywhere while
  you type.
- Automatic updates: installed copies look for a newer release shortly after
  start and every six hours, download it in the background, and install it as
  the app quits. The next launch opens the release notes in a tab. Every update
  file is signed, and a file whose signature does not match is refused.
- Settings > About: the running version, a button to check for updates, a
  switch to turn automatic updates off, and a link to the open-source licenses.
- Sidebar layouts (right, the new default, or left) with a hotbar across the
  top, next to the classic top bar in relaxed or compact size.
- Customize toolbar: drag the address bar, buttons, the workspace switcher and
  spacers between the hotbar and the sidebar, or remove them.
- Collapsible bar that floats back over the page when the cursor nears its
  window edge.
- Workspaces: named groups of tabs with their own icon and color, saved in the
  session; tabs can be moved between them.
- Private tabs and private windows. Nothing from them is kept on disk or saved
  in the session.
- Theme editor: presets, up to three background colors as a gradient, color
  strength, textures, accent color, light and dark modes.
- Frosted glass behind the floating bar on macOS, and blurred menus.
- Double-click an empty part of the bar to expand the window; drag it to move.
- `THIRD-PARTY-NOTICES`, `PRIVACY.md`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`
  and this changelog.
- Windows: the uninstaller offers to delete settings and browsing data, and the
  installer shows the license.

### Changed

- The interface is rebuilt on a design and motion system, documented in
  `docs/DESIGN.md`: the page sits on a rounded card that glides to its new size,
  tabs grow in and fold away, and icons are line icons.
- The address bar shows the site's host emphasized and hides `https://` while
  resting.
- Restored tabs load only when first opened.
- macOS: Wisp identifies itself to sites as the installed version of Safari
  (it is the same engine), so sites such as Google serve their modern pages.
- Windows: the publisher shown under Installed apps is "Siimas" (it read
  "github").

### Security

- Hotkey messages coming from pages are checked more strictly and are ignored
  while Settings covers the page.
- Ad-block host matching can no longer be sidestepped with crafted URLs (user
  information, ports, backslashes).

## [0.1.0] - 2026-11-16

First release.

### Added

- Tabs around the system web engine (WebKit on macOS, WebView2 on Windows).
- Settings page that applies changes live; every hotkey can be rebound.
- Ad and tracker blocking from a built-in host list plus your own hosts.
- Session restore.
- Installers: `.dmg` for macOS (Apple Silicon and Intel) and a setup program
  for Windows.

[Unreleased]: https://github.com/Siimas/wisp-browser/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/Siimas/wisp-browser/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/Siimas/wisp-browser/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/Siimas/wisp-browser/releases/tag/v0.1.0
