# Privacy

Wisp has no accounts, no sync, no telemetry, no analytics and no crash
reporting. There is no Wisp server: the project does not receive anything about
you or what you browse.

This page lists every network request the app makes on its own, what stays on
your computer, and what is outside Wisp's control. It describes the code in
this repository; the places to check are named so you can verify it.

## Requests Wisp makes by itself

There are two.

**Update checks** (`src/update.rs`). A copy installed from a release asks
GitHub for
`https://github.com/Siimas/wisp-browser/releases/latest/download/latest.json`
shortly after it starts and every six hours, and downloads the new version from
GitHub when there is one. Wisp adds no identifier to these requests; GitHub
sees what any web server sees (your IP address and the time). Turn it off with
Settings > About > Update automatically; after that a check happens only when
you press **Check for updates**. Private windows never check. Copies you build
yourself never check.

**Site icons** (`src/ui.html`, the `favicon.ico` line). For each tab, the tab
list asks that tab's own site for `https://<host>/favicon.ico`. The request
goes only to a site you already have in a tab, never to a third party. Note
that after a restart this includes restored tabs you have not opened yet.
Private tabs show a fixed icon and make no such request.

## Requests that follow from what you do

- **Pages you visit** are loaded by the system web engine, and those sites see
  what any browser shows them. A tab that was unloaded to save memory (Settings >
  Tabs and browsing > Performance) loads its page again from the site when you
  come back to it.
- **Searches**: text typed in the address bar that is not an address is sent to
  your search engine when you press Enter (Google by default, changeable in
  Settings). Nothing is sent while you type: the suggestions under the address
  bar come only from your own open tabs, bookmarks and history
  (`src/store.rs`), never from a search engine.
- **Downloads**: a file you download is fetched by the system web engine from
  the site that offers it. Wisp makes no request of its own for it.
- **Find in page** searches the page on your computer. What you type in the find
  bar is not sent anywhere and is not kept.
- **Home page**: a new window opens your home page (Google by default).
- **Release notes and licenses**: after an update, and when you press View next
  to Open-source licenses, Wisp opens a page on github.com in a tab.

## What Wisp does not do

- It does not download block lists: the ad and tracker list is built into the
  app, and matching happens on your computer.
- It does not send your history, tabs, settings or searches anywhere.
- It does not contain third-party SDKs.

## What is stored on your computer

- Settings (`config.json`), the open tabs of your last session
  (`session.json`: addresses and titles) and the last version run (`version`),
  in `~/Library/Application Support/Wisp/` on macOS and `%APPDATA%\Wisp\` on
  Windows.
- History (`history.json`): for each page visited, its address, title, number
  of visits and time of the last visit. At most 20,000 addresses are kept; the
  one visited longest ago is dropped first. Written at most every 30 seconds
  and when the app quits. Turn it off with Settings > Behavior > Keep history;
  delete entries, or everything, from History (Clear browsing data). Same
  folder as the settings.
- Bookmarks (`bookmarks.json`): address, title and the time each was added.
  Same folder.
- Cookies, cache and site storage, kept by the system web engine. History >
  Clear browsing data can remove these too. See
  [Uninstall](README.md#uninstall) for the folders.
- Files you download, in your Downloads folder. The list of downloads shown in
  the toolbar (file name, the site it came from, and whether it finished) is
  held in memory only and is gone when Wisp quits.
- Unloaded tabs: before unloading a tab, Wisp asks the page on your computer
  whether text was typed into it (to keep it loaded if so) and where it is
  scrolled to. The scroll position is held in memory only, to put the page back
  there, and is gone when Wisp quits. Nothing about unloading is written to disk
  apart from its two settings in `config.json`.
- Private tabs and private windows keep nothing on disk and are left out of the
  saved session and of history. A private window can read your history and
  bookmarks to suggest addresses, but cannot change them, and private tabs are
  never offered as suggestions.
  your Downloads folder like any other, and appears in the downloads list until
  Wisp quits.

Wisp stores nothing else. Removing these folders removes everything except the
files you downloaded.

## Outside Wisp's control

- Wisp renders pages with WebKit on macOS and Microsoft Edge WebView2 on
  Windows. These are system components with their own behavior and settings,
  covered by Apple's and Microsoft's privacy policies. Wisp does not switch on
  any reporting in them, but it does not audit them either.
- On macOS, Wisp identifies itself to sites as the installed version of Safari.
- Installers and updates are hosted by GitHub, whose privacy statement applies
  to downloads.

## Changes

Any change to what Wisp sends is recorded in [CHANGELOG.md](CHANGELOG.md).
Questions: open an issue at <https://github.com/Siimas/wisp-browser/issues>.
