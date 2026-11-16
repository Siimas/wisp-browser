# Extensions spike: can Wisp run real browser extensions on macOS?

Time-boxed experiment, November 2026, on macOS 26.5.2 (25F84) with the system WebKit.
The code is experimental and off by default (see "The experiment code" at the end).

## Verdict for NordPass: does not work today

**The NordPass extension from the Chrome Web Store loads in Wisp but cannot get past its
own start-up.** It requires two Chrome-only APIs that WebKit does not have: `chrome.offscreen`,
which its Chrome build uses to run the worker pool that decrypts the vault, and `chrome.idle`,
which it uses for auto-lock. WebKit also lacks `runtime.getContexts` and `chrome.privacy`.
Up to that point it works:
- its background starts;
- its content script recognises a login form and puts the NordPass icon in the email
  field, in the locked, signed-out state;
- its toolbar popup opens, but it never gets past a loading spinner, so there is
  no sign-in screen to reach.

**The build of NordPass designed for this API, its Safari extension, could not be tested.**
It ships only inside the Mac App Store app, which needs the owner's Apple ID. Its code takes a
different path from the Chrome build: Web Workers instead of `offscreen`, and native messages
instead of `idle`. That makes it the realistic route. Bitwarden's Safari build, which I could
test, shows the cost of that route. Its native messages, which Safari would deliver to the
vendor's own app, arrive at Wisp instead, so Wisp would have to answer each vendor's private
protocol itself.

What could change this:
- **Apple ships `offscreen` in WebKit.** A build flag for it, `WK_WEB_EXTENSIONS_OFFSCREEN`,
  exists in WebKit's main branch ([bug 322782](https://bugs.webkit.org/show_bug.cgi?id=322782),
  August 2026). It is not in macOS 26.5.
- **The owner's own test of the Safari build**, following the steps at the end of this report.

The desktop app is not part of the picture. NordPass's current extension signs in and syncs
on its own over the cloud (HTTPS plus MQTT over `wss://*.nordpass.com:8884/8885`). The
installed desktop app (Electron, 7.12.15) has no native-messaging host and no local server
for extensions. The Chrome Web Store marks the older "desktop app version" extension as
legacy: it is retired on 1 January 2027, and its 7.12.15 code behaved the same as the current 7.12.16 in every test here.

## What was proven, extension by extension

Each run used a throwaway `HOME`, a local test page, the debug window capture
(`WISP_SHOT`), and a probe that reads the page DOM. Every permission was granted automatically.

| Extension (source, version) | Loads | Unsupported keys, missing APIs | Background | Content scripts | Popup | Login form | Errors |
|---|---|---|---|---|---|---|---|
| **Wisp Hello** (written for this test, MV3) | yes | none | runs (100–240 ms) | inject (banner on page) | opens, shows storage and `tabs.query` result | n/a | none. Messaging works sync, `return true` and promise style; `sender.tab` resolves to Wisp's tab |
| **Wisp DNR** (one `declarativeNetRequest` rule) | yes | none | none | n/a | n/a | n/a | blocks `googletagmanager.com` from the first request |
| **uBlock Origin Lite** 2026.1116.1650, [GitHub releases](https://github.com/uBlockOrigin/uBOL-home/releases), Chromium and Safari zips | yes | none reported | runs (115–490 ms) | n/a (none declared) | opens, full UI ("filtering mode: optimal") | n/a | **Its default lists did not block anything, even after 40 s**, and WebKit reported no error. The same EasyPrivacy list (56,583 rules) packed into my own extension blocked within 4 s, so WebKit's DNR copes; the cause is in uBOL's own set-up path. Not isolated (time box). |
| **Bitwarden**, Chrome build 2026.10.2, [GitHub](https://github.com/bitwarden/clients/releases) | yes | dropped: `idle`, `offscreen`, `sidePanel`, `webRequestAuthProvider`, `clipboardRead` | **fails** ("background content failed to load due to an error"). With a Safari-style user agent (see note) it runs | none seen | opens, then crashes (`this.device` is null: Bitwarden identifies the browser by user agent) | no | Background probably dies on `webRequest.onAuthRequired` with `asyncBlocking` (WebKit's `webRequest` only observes); inferred from code, not proven |
| **Bitwarden Safari extension** (`safari.appex` from Bitwarden desktop 2026.10.4, extension 2026.10.1, MV2 persistent), loaded **as an app-extension bundle** | yes | none reported | runs (260–320 ms) | none seen while signed out | **opens and shows Bitwarden's real signed-out screen** ("Log in", "Create account") once a Safari-style user agent is set | not shown signed out | Sends native messages to its own app (`sleep`, used as a timer) and opens a port to `com.8bit.bitwarden`. Wisp refuses them, and it retried **104,061 times in 16 s** |
| **NordPass**, current Chrome Web Store build 7.12.16 (`eiaeiblijfjekdanodkjadfinkhbfgcd`) | yes | dropped: `idle`, `offscreen`, `privacy`; missing `runtime.getContexts` | runs (400–950 ms) | **inject**: `<nordpass-icon>` in the email field | opens, **spinner forever**; popup logs `SyncoRedux Disconnected due to an error` every second | **yes, detects it** (icon in the field, locked state) | No native messages, no requests to the desktop app. With stubs for `idle` and `getContexts` added to a test copy the disconnect loop stops, but the spinner stays (the `offscreen` decrypt pool cannot be stubbed) |
| **NordPass (legacy)** 7.12.15 (`fooolghllnmhmmndgjiamiiodkpenpbb`) | yes | same as above | runs | inject (same icon) | spinner | yes | same code as the current build |
| **NordPass for Safari** | not tested | | | | | | Mac App Store only; not installed on this Mac |

Screenshots of these runs are in the session notes folder
(`side-session-notes/local_f87b5624-…/extensions-spike-shots/`).

Notes that apply to all extensions:
- **User agent.** Extension pages and backgrounds get WebKit's bare user agent, which has no
  `Safari/` in it. Bitwarden (and likely others) choose their code path by user agent and
  crash without one. The spike sets a Safari-style application name on the controller's base
  configuration, matching what Wisp tabs already send.
- **Silent drops.** WebKit silently drops manifest permissions it does not support; nothing
  appears in `errors`. `chrome.idle`, `chrome.offscreen`, `chrome.privacy`,
  `chrome.sidePanel`, `runtime.getContexts` and `runtime.connectNative` were all `undefined`
  in extension pages.
- **WebAssembly.** WebKit serves `.wasm` from `webkit-extension://` without the
  `application/wasm` type. Bitwarden logs a warning and falls back to a slower load.
- **The app-extension-bundle route works.** `WKWebExtension(appExtensionBundle:)` loaded
  Bitwarden's signed `.appex` straight out of its app. However, its native messages still go
  to Wisp's delegate, not to the vendor's extension handler.

## How it works in Wisp (the sequence)

1. Create `WKWebExtensionController` with `WKWebExtensionControllerConfiguration.default`, set
   its delegate, and give the configuration's base `webViewConfiguration` the Safari-style
   user agent.
2. Create a `WKWebExtension` from either source, both asynchronous:
   - an unpacked folder or zip: `extensionWithResourceBaseURL:completionHandler:`;
   - an `.appex`: `extensionWithAppExtensionBundle:completionHandler:`.
3. Create the context: `WKWebExtensionContext(forExtension:)`. Grant permissions and match
   patterns with `setPermissionStatus:forPermission:` and `…forMatchPattern:`. Then
   `controller.loadExtensionContext:error:`, then `loadBackgroundContentWithCompletionHandler:`.
4. For each tab, hand wry a `WKWebViewConfiguration` that carries the controller, through
   `WebViewBuilderExtMacos::with_webview_configuration`. **Gotcha:** wry then skips its own
   data-store choice, including incognito, so the configuration must set the data store
   itself. Private tabs simply never get the controller.
5. Wrap each tab in an object implementing `WKWebExtensionTab` (web view, title, URL,
   selected, window). Wrap the window in one implementing `WKWebExtensionWindow`. Report tab
   events to the controller: `didOpenTab`, `didActivateTab`, `didCloseTab`, `didOpenWindow`,
   `didFocusWindow`.
6. Implement the delegate: open windows, focused window, permission prompts (three kinds),
   present the action popup (`action.popupWebView` is a ready `WKWebView` to place in a
   popover), open tab or window, open options page, and native messaging (`sendMessage` and
   `connectUsingMessagePort`). Toolbar button: call `context.performActionForTab:`.

## What proper support would involve

- **Toolbar.** An extension button area in `ui.html`: icons (`action.iconForSize`), badge
  text, enabled state, live updates from `didUpdateAction`, and a real `NSPopover` for the popup.
- **Install flow and management.** Install from a folder, a zip or a CRX (strip the CRX
  header), or from an `.appex` found in `/Applications`. Updates: there is no store, so Wisp
  must fetch updates itself or leave them to the vendor's app. Enable, disable and remove,
  plus a settings page.
- **Stable identity.** Each extension needs a stable `uniqueIdentifier` and base URL. The
  spike gets a random one per launch, so `storage` starts empty each time and a sign-in would
  not survive a restart.
- **Permission UI.** Prompts at install and on request (all three delegate prompts),
  per-site access ("on click / on this site / on all sites"), and remembered decisions.
- **The rest of the delegate.** Tabs and windows created by extensions (`tabs.create`, which
  NordPass and Bitwarden use for onboarding), options pages, context-menu items
  (`menuItemsForTab`), keyboard commands (`performCommandForEvent`), and `didMoveTab` and
  `didChangeTabProperties` notifications.
- **Native messaging.** Wisp would have to answer each vendor's private native-message
  protocol (NordPass: heartbeat timer, clipboard, in-app purchase; Bitwarden: `sleep` timer,
  biometrics), or broker a connection to the vendor's app. There is no public API to call an
  `.appex`'s own handler.
- **Shims for missing APIs** where vendors' Chrome builds need them. `offscreen` cannot
  realistically be shimmed from outside.
- **Profile and privacy.** Extension data in the right profile folder, wiped with the
  profile, and kept out of private windows; private windows can then never see extensions.
- **Inspection.** A developer toggle for inspecting extension backgrounds (the spike sets
  `inspectable`).

## Windows and Linux (from documentation, not built)

- **Windows (WebView2).** Microsoft documents `CoreWebView2EnvironmentOptions.AreBrowserExtensionsEnabled`
  and `CoreWebView2Profile.AddBrowserExtensionAsync` (unpacked folders only), and wry 0.57
  exposes both (`with_browser_extensions_enabled`, `with_extensions_path`). The engine is
  Chromium, so API coverage, `offscreen` included, should be far closer to Chrome's. The
  catch is in Microsoft's docs: extensions "can be loaded and used but will have missing UI
  entry points" (no toolbar icon, badge or popup), so Wisp would have to draw the button and
  host the popup page itself, with no documented way to fire `action.onClicked`. Native
  messaging is not documented either way. The Chrome NordPass build has a decent chance here,
  but this is inferred, not tested.
- **Linux (WebKitGTK).** WebKitGTK has no browser-extension API. Its "web extensions" are
  native web-process plugins, which is what wry's Linux `with_extensions_path` loads. GNOME
  Web carries its own partial WebExtensions layer behind a setting; its developers describe
  it as unfinished and not actively developed. Extensions on Linux would mean writing that
  layer, or not offering them.

**Share of popular extensions likely to work on macOS.**
- Content-script and popup extensions using `storage`, `tabs`, messaging, `scripting` and
  DNR should mostly work: dark-mode tools, translators' page helpers, simple blockers, and
  helpers that need nothing else.
- Extensions that depend on `offscreen`, `idle`, `sidePanel`, blocking `webRequest`,
  `privacy`, `downloads`, `history`, `bookmarks` or `identity` will fail or degrade. That
  group includes most password managers' Chrome builds and many productivity extensions.
- A rough guess, not measured: half or fewer of the Chrome top 50 would run unmodified.
- Vendors who ship a Safari build fare better, but Safari builds lean on native messaging
  to their own apps.

## Security notes

- **What an extension can reach.** An extension with `<all_urls>` (NordPass, Bitwarden and
  uBOL all ask for it) runs code in every page of every non-private tab. In NordPass's case
  that includes main-world scripts. It can also read URLs and titles of all tabs and make
  network requests to its own servers. WebKit enforces the grants, but **the host decides
  them**: the spike grants everything, which is exactly what a real build must not do.
- **Where permissions are decided.** Each permission and match pattern is set per context by
  Wisp (`setPermissionStatus…`), and requests at run time come through the delegate's prompts.
  So Wisp owns the consent UI and can offer per-site or click-to-run grants.
- **Private tabs.** These can be kept extension-free, and it was verified: a private window
  loading the same test page got no content script, because its tabs are built without the
  controller. If ever wanted, `WKWebExtensionWindow.isPrivate` and
  `hasAccessToPrivateData` exist for an opt-in per extension.
- **Native messaging.** This is a privileged bridge. Whatever Wisp answers becomes part of
  the vendor's trust model, and answering a password manager's private protocol means Wisp
  sits inside its security boundary.
- **Data location.** WebKit ignores `HOME`, so the spike's runs wrote web data (no
  credentials, nothing signed in) to `~/Library/WebKit/wisp`. That folder belongs to the
  unbundled debug binary and is separate from the installed Wisp's `io.github.siimas.wisp`.
  It is safe to delete.
- **The desktop-app handoff.** NordPass's desktop app keeps an allow-list of extension
  origins (`chrome-extension://<NordPass ids>`, `moz-extension:`, `safari-web-extension:`)
  for handing a sign-in to the extension. Wisp's `webkit-extension://` origin is not on it,
  so that convenience would not work from Wisp.

## Cost (measured, debug build)

Memory is Wisp plus the WebKit processes it started, measured 11 s after launch with one test
tab, as the minimum of five runs (other apps add noise, never subtract):

| Setup | Total | vs. none |
|---|---|---|
| No extensions | 183 MB (5 processes) | |
| Wisp Hello | 207 MB (6 processes) | +24 MB |
| NordPass | 219 MB (6 processes) | +36 MB |
| uBlock Origin Lite | 558 MB (6 processes) | +375 MB, almost all in Wisp's own process (444 MB), where WebKit converts uBOL's 111k rules |

Start-up: reading an extension takes 130–760 ms (NordPass is 12 MB) and runs alongside window
and tab creation without blocking either. Loading the context takes 0–4 ms. The background is
ready 100–950 ms later. Wisp's own built-in blocker stays the cheaper way to block ads.

## Steps for the owner's own sign-in test

Do this only with an account you are happy to sign in to on a test build. Nothing here was
signed in during the spike.

1. Install **NordPass for Safari** from the Mac App Store (the page linked from
   nordpass.com/download/safari). Do not enable it in Safari; Wisp loads it directly.
2. Check out the branch `claude/extension-spike-85b466` and run `cargo build`.
3. Find the extension bundle:
   `find /Applications -path "*NordPass*" -name "*.appex" -path "*PlugIns*"`.
4. Start Wisp with it, from the repository folder:

   ```bash
   WISP_EXTENSIONS="/Applications/<NordPass app>/Contents/PlugIns/<extension>.appex" WISP_EXT_PRESS=5 ./target/debug/wisp 2>&1 | tee /tmp/wisp-nordpass.log
   ```

   `WISP_EXT_PRESS=5` opens the extension's popup, top right, after 5 seconds; there is no
   toolbar button yet. The popup is a normal page: click and type in it.
5. Sign in in the popup. Watch the terminal for `NATIVE MESSAGE` and `NATIVE CONNECT` lines.
   Wisp refuses them, so expect auto-lock and clipboard to misbehave, and possibly a retry loop.
6. Open a site you have a login for, focus its username field, and check whether the NordPass
   icon and autofill appear.
7. Optional comparison: the Chrome Web Store build. Unpack the CRX downloaded from
   `https://clients2.google.com/service/update2/crx?response=redirect&prodversion=142.0&acceptformat=crx2,crx3&x=id%3Deiaeiblijfjekdanodkjadfinkhbfgcd%26uc`
   with `unzip`, and pass the folder as `WISP_EXTENSIONS`. Expect the spinner.
8. Afterwards, sign out in the popup, quit, and delete `~/Library/WebKit/wisp`. Sign-in does
   not survive a restart anyway: the spike gives the extension a new identity each launch.

## What I could not test, and why

- **NordPass for Safari:** distributed only through the Mac App Store, which needs the
  owner's Apple ID.
- **Signing in, autofill, the vault:** forbidden for this test, and blocked earlier anyway by
  the popup.
- **uBOL's blocking failure:** cause not isolated within the time box.
- **Bitwarden Chrome build's popup** after its background started: the toolbar press did not
  open it; not investigated.
- **Windows and Linux:** documentation only.
- **The toolbar badge:** the test extension set a badge text but the delegate never saw it;
  not followed up.

## The experiment code

- `src/extensions.rs` (macOS only, marked EXPERIMENTAL) holds the controller, the delegate
  and the tab and window objects. It is inert unless `WISP_EXTENSIONS` is set (paths separated
  by `:`; folder, zip or `.appex`).
- Changes in `src/main.rs`:
  - the configuration hook in `build_webview`;
  - tab open, activate and close notifications;
  - debug-only hooks: `WISP_EXT_PRESS`, `WISP_PROBE` (+`_AT`), `WISP_POPUP_PROBE` (+`_AT`),
    `WISP_SHOT_AT`.
- `experiments/extensions-spike/` holds the test extensions, the local test pages, the run and
  memory scripts, and the API shim used on a NordPass test copy.

This is not a feature: it grants every permission without asking, logs extension messages to
stderr, and has no UI. Keep it on the branch for reference; a real implementation would start
from the sequence above, not from this file.
