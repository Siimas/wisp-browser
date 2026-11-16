# Extensions spike (EXPERIMENTAL)

Test material for docs/EXTENSIONS-SPIKE.md. Not part of Wisp.

- `hello/` test extension: content script, background, popup, storage, messaging.
- `dnr/` one declarativeNetRequest rule.
- `site/` local test pages. Serve with `python3 -m http.server 18931 --bind 127.0.0.1 --directory site`.
- `run.sh NAME URL VAR=value...` runs the debug build with a throwaway HOME and prints
  `wisp-ext:` and probe lines. Output goes to `$SPIKE_OUT` (default /tmp/wisp-spike).
  Example: `./run.sh hello http://127.0.0.1:18931/login.html WISP_EXTENSIONS=$PWD/hello WISP_EXT_PRESS=4 WISP_SHOT_AT=7`
- `mem.sh` measures memory of a run. `probe.js` is the DOM probe for login pages.
  `shim.js` stands in for `chrome.idle` and `runtime.getContexts` (used on a test copy of NordPass).

Note: WebKit ignores HOME; web data from debug runs lands in ~/Library/WebKit/wisp.
