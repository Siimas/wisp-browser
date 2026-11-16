# How Wisp performs against other browsers

Measured on 16 November 2026, on one machine, by `bench/bench.py`. Raw numbers: [`performance-data.json`](performance-data.json).

## Findings

| | Wisp 0.2.0 (`73f906f`) | Zen 1.24.1b | Chrome 157 | Safari 26.5.2 |
|---|---|---|---|---|
| App on disk | **1.8 MB** ¹ | 591 MB | 738 MB ² | 36 MB ¹ |
| Download | **0.9 MB** | 228 MB | 270 MB | ships with macOS |
| Launch to simple page loaded (median of 5) | **0.37 s** | 1.01 s | 0.49 s | not measured |
| Memory, one simple tab (median of 3) | **188 MB** | 716 MB | 239 MB | not measured |
| Idle CPU, one simple tab (% of one core) | 20.1 % | **0.02 %** | 0.25 % | not measured |
| Memory, same 10 real sites, all loaded (median of 3) | 2,448 MB | 2,218 MB | **1,152 MB** | not measured |
| Idle CPU with those 10 tabs (% of one core, median of 3) | 32.0 % | 31.8 % | **13.0 %** | not measured |
| Speedometer 3.1 (median of 3, higher is better) | 45.2 | 46.0 | **59.8** | not measured |

Memory is the whole process tree's *physical footprint* (what Activity Monitor shows as "Memory"), helpers included. ¹ Wisp and Safari both run on the copy of WebKit that is part of macOS, which neither bundle includes. ² Chrome's folder is 2,195 MB because it keeps two older versions of its framework from past updates; 738 MB is the current version alone.

**Since this was measured.** Everything above is Wisp at `73f906f`, the top of the `auto-update` stack. While the measurements ran, that whole stack and five more pull requests (history and bookmarks, page tools, tab basics, brand themes, spacing) were merged into `main` (`6b110d9`). Wisp rebuilt from `main` and re-measured (size and one tab only, three runs):

| Wisp | `73f906f` | `main` (`6b110d9`) |
|---|---|---|
| App / binary | 1.8 MB / 1.52 MB | 2.0 MB / 1.69 MB |
| Download (zip) | 0.91 MB | 0.98 MB |
| Memory, one simple tab | 188 MB | 268 MB |
| Idle CPU, one simple tab | 20.1 % | 10.3 % (runs 16.6, 10.3, 10.3) |

The 80 MB increase is all in the two web-content processes (81 → 156 MB), and the page's own process was unchanged before, so it is the interface page that grew. The progress-bar animation is still in `main` (`src/ui.html`, `#progress::after`), so the idle cost is still there, though lower. Neither change was investigated further. With 268 MB, Wisp's one-tab memory is now *higher* than Chrome's 239 MB.

**Where Wisp wins**

- **Size.** Wisp is 250 to 300 times smaller to download than Zen or Chrome, and its app is under 2 MB. This is real, but only because the engine is the system's: the comparison is "Wisp + WebKit already on every Mac" against browsers that ship their own engine.
- **Launch.** Wisp has a page loaded about 0.6 s sooner than Zen and about 0.1 s sooner than Chrome. Neither of those was a true cold start (see [Cold start](#cold-start)).
- **Restored sessions.** Wisp does not load a restored tab until you open it. Ten restored tabs, with only the active one loaded, cost 281 MB. Zen does the same (`restore_on_demand`), so this is an advantage over Chrome's default, not over Zen.
- **One tab.** With one simple page Wisp's whole tree is 188 MB, against 239 MB for Chrome and 716 MB for Zen. The Wisp process itself is only 22 MB; the rest is WebKit's helpers, counted in full. On `main` this is 268 MB, which is more than Chrome (see above).

**Where Wisp loses**

- **Idle CPU, because of a bug in Wisp's own interface.** The progress bar under the address bar is hidden with `opacity: 0` when no page is loading, but its infinite `sweep` animation keeps running. That animation moves `left`, so WebKit lays out and repaints the interface every frame for ever. With one idle tab Wisp uses 20 % of a core where Zen uses 0.02 % and Chrome 0.25 %, and almost all of it is this animation (interface web content + GPU process + Wisp itself). Hiding the element dropped the whole tree from 19.3 % to 1.5 % in an A/B test (see [Idle cost](#idle-cost)). It is also why Wisp's ten-tab idle CPU is no better than Zen's and more than twice Chrome's.
- **Memory with many loaded tabs.** With the same ten sites loaded, Wisp used 2.4 GB against Chrome's 1.2 GB and Zen's 2.2 GB. Almost all of it (2.2 GB) is in WebKit's 14 web-content processes. Chrome's lower number is *not* from tab discarding: all its renderers were still alive at the 100 s sample, so its page processes are simply smaller (869 MB for ten sites against WebKit's 2,178 MB here). Why WebKit's are bigger was not investigated. Wisp also never puts a loaded tab to sleep, so with more tabs open for longer the gap can only grow, while Chrome's Memory Saver and Zen's manual unload can shrink theirs. Much of WebKit's footprint here is memory macOS has already *compressed* (resident memory is only 0.76 GB), so it costs less real RAM than the footprint suggests, but compressed memory still counts against the system and is what Activity Monitor shows.
- **Engine speed.** Speedometer is 45 in Wisp and 60 in Chrome. Wisp gets whatever the system WebKit gives it; Zen scored the same as Wisp (46), with a wider spread.
- **Features.** Wisp has no tab sleeping, no memory or energy saver, no per-tab resource view and no preloading (table below). Its ad blocking is a 157-host list, not a filter list.

**What would help most**

1. **Stop the hidden progress bar animating** (`src/ui.html`, `#progress::after`): give the animation only while `#progress.on`, or use `display: none` / `animation-play-state: paused` when off, and animate `transform` instead of `left` so it stays on the compositor. This is a one-line change and is the biggest single win measured here: about 18 % of a core at idle (19.3 % → 1.5 % in the A/B test), all the time the window is on screen.
2. **Unload background tabs.** Drop the `WebView` of a tab not shown for N minutes (keeping its URL and title, as a restored tab already is) and rebuild it on select. The lazy-restore path in `App::select` already does the rebuilding, so this is mostly a timer and a `tab.webview = None`. This would not have changed the ten-tab number above (Chrome did not discard anything in 100 s either), but it bounds memory over a long day with many tabs, which is what Memory Saver is for.
3. **Then look at the collapsed bar.** With the bar collapsed the event loop wakes every 25 ms to poll the cursor (`POLL` in `src/main.rs`). Measured, it adds about 1 % of a core (Wisp process 3.3 % → 4.5 %). Small next to item 1, but it will be most of what is left once item 1 is fixed; tracking the mouse with an `NSTrackingArea` or a window-edge event instead of polling would remove it.
4. **Show per-tab cost.** Even a list of web-content processes and their footprint in settings would let people find the heavy tab; WebKit gives no per-tab process id through public API, so this is harder than it sounds.

**Not measured, and why**

- **Safari: everything but size.** Safari can only be driven through `safaridriver`, which needs "Allow remote automation" turned on in Safari's settings. It is off here and the brief rules out changing browser settings. Safari also cannot be started as a separate instance on a temporary profile, so measuring it would have meant using the owner's real profile. Wisp runs on the same system WebKit as Safari 26.5.2, so Wisp's Speedometer number is a reasonable guess at Safari's engine, but it is not a Safari measurement and is not in Safari's column.
- **Firefox, Arc, Brave, Edge**: not installed. Only Safari, Zen and Chrome are in `/Applications`.
- **JetStream and MotionMark**: not run, for time.
- **True cold start** (empty file cache) needs `sudo purge`, which this session cannot run.

**Noise.** All of this was measured on the owner's laptop *while the owner was using it*, with their own Zen and Chrome open. Browser windows were opened in the background (`open -g`), but other work on the machine competes for CPU and memory. The ten sites are live and change between runs. Treat differences under about 10 % as noise.

### What the interface's looks cost

Measured 16 November 2026 for low-power mode, after the progress-bar fix: release build, one simple
tab, whole tree 25 s after launch, idle CPU over 30 s, two runs each, each feature switched off on
its own (through custom CSS). Same machine and accounting as above; a short script, not `bench.py`.

| `main` (`a88dd6a`) | Whole tree | Interface web content | Idle CPU |
|---|---|---|---|
| As shipped | 163-212 MB | 118-145 MB | 0.1-0.5 % |
| No grain texture | 116 MB | 67 MB | 0.2-0.3 % |
| Flat paint instead of the gradient | 184-187 MB | 134 MB | 0.1 % |
| Hairline-only shadows | 165-171 MB | 117 MB | 0.2 % |
| All of these off | 116 MB | 67 MB | 0.1 % |
| Grain layer `position: absolute` instead of `fixed` | 128 MB | 67 MB | 0.2 % |
| Bar collapsed (second web view + cursor poll) | 199-201 MB | 149 MB | 0.8-1.3 % |

The grain was the whole cost, and only because its layer was `position: fixed` under a blend
mode: as `absolute` it draws the same pixels (largest difference 3 of 255: two debug-build
window captures, dark theme, `fixed` and `absolute`, compared per pixel with Pillow's
`ImageChops.difference` over a patch of bare paint) at no measurable cost,
so that is now the default. With it, and the cursor checked less often when far from the edge:

| This branch | Whole tree | Interface web content | Idle CPU |
|---|---|---|---|
| As shipped | 115-121 MB | 68 MB | 0.1-0.4 % |
| Bar collapsed | 145-152 MB | 98 MB | 0.3-1.0 % |
| Low-power mode on (either layout) | same as above | same | same |

Low-power mode saves nothing at rest: what it removes (glass blur, animation frames, the page
resizing frame by frame) costs only while something is shown or moving. The collapsed bar's
second web view is about 30 MB; it is kept, because building it on demand would delay the
reveal. About one run in six was cut short because the owner closed the test window while
working on the machine (not a crash); those runs were dropped.

## Machine

MacBook Pro (Mac16,1), Apple M4, 10 cores, 16 GB, macOS 26.5.2 (25F84). On mains power.

| Browser | Version | Engine |
|---|---|---|
| Wisp | 0.2.0, commit `73f906f` (`auto-update` branch), `cargo build --release` + `cargo packager --release --formats app` | System WebKit (Safari 26.5.2's) |
| Zen | 1.24.1b | Gecko, Firefox 159.0 |
| Google Chrome | 157.0.8103.78 | Blink / V8 |
| Safari | 26.5.2 | WebKit (size only) |

## Method

`bench/bench.py` (Python standard library only) runs a small local web server and starts each browser as a **separate instance on a throwaway profile**:

- Wisp: `open -n` on the packaged app with `HOME` and `CFFIXED_USER_HOME` pointing at a temporary folder, so both Wisp's own files and WebKit's website data go there. Tabs are preloaded through `session.json`.
- Zen: `-profile <tmp> -no-remote`, with a `user.js` in that profile that only skips the welcome screen, the default-browser check and update checks.
- Chrome: `--user-data-dir=<tmp> --no-first-run --no-default-browser-check`.

The script identifies the new instance by process id (and for Zen and Chrome, by its profile path in the command line) and stops it with `SIGTERM` to that process id only. Nobody's running browser was touched, and no browser settings or default-browser status were changed. Nothing was signed in.

**Which processes count.** A browser's process tree is every descendant of its main process, plus every process whose *responsible process* is the browser (`responsibility_get_pid_responsible_for_pid`, which is how Activity Monitor groups them). The second rule is what catches WebKit's helpers: `com.apple.WebKit.WebContent`, `.GPU` and `.Networking` are XPC services started by launchd, not children of Wisp, but macOS records Wisp as responsible for them. The same rule picks up `com.apple.audio.SandboxHelper` and `MTLCompilerService` for Wisp, and Apple's `VTDecoderXPCService` / `VTEncoderXPCService` video services for Zen and Chrome. All of these are counted. Safari's helpers would be attributed the same way.

**Memory metric.** `proc_pid_rusage` per process: *physical footprint* (`ri_phys_footprint`, Activity Monitor's "Memory" column, which includes compressed memory and GPU memory the process owns) is the headline. Resident size (RSS) is recorded too but double-counts shared libraries in every process, which inflates multi-process browsers (Zen's ten-tab RSS is 5.6 GB against a 2.2 GB footprint). Each figure is the median of three samples two seconds apart.

**CPU metric.** User + system CPU time of the whole tree over 60 seconds, as a percentage of one core.

## Results

### Size on disk

| | App bundle (`du`) | Download |
|---|---|---|
| Wisp | 1.8 MB (binary 1.52 MB) | 0.91 MB zip of this build; the published 0.1.0 `.dmg` is 0.94 MB (aarch64) |
| Safari | 36.1 MB | ships with macOS |
| Zen | 590.8 MB | 228.3 MB (`zen.macos-universal.dmg`, GitHub release 1.24.1b) |
| Chrome | 2,195 MB on disk, 738 MB without the two stale framework versions | 270 MB (universal `googlechrome.dmg` from dl.google.com) |

Not like-for-like: Wisp and Safari leave out WebKit, which lives in macOS's system volume and is shared by every app that shows web content. Zen and Chrome are universal (Intel + Apple silicon) downloads; Wisp's is Apple silicon only.

### Cold start

Time from `open` until a local simple page's `load` event reached the bench server. The first launch, on a brand-new profile, is reported apart; then five launches on that profile.

| | Median | Min | Max | Runs (s) | First launch, fresh profile |
|---|---|---|---|---|---|
| Wisp | **0.37 s** | 0.31 | 0.45 | 0.33, 0.31, 0.38, 0.45, 0.37 | 0.36 s |
| Zen | 1.01 s | 0.99 | 1.20 | 1.00, 1.01, 1.05, 1.20, 0.99 | 1.29 s |
| Chrome | 0.49 s | 0.46 | 0.69 | 0.49, 0.46, 0.51, 0.48, 0.69 | 0.82 s |

Not like-for-like: these are *warm* starts. The owner's own Zen and Chrome were running, so their code was already in the file cache, and WebKit is always in memory because macOS uses it everywhere. A true cold start needs `sudo purge` first. Wisp's time includes compiling its ad-block rules before the first tab is created (tabs wait for `AdblockReady`).

### Memory

#### One simple tab

Sampled 100 seconds after launch, on a profile that had already been through one unrecorded launch (so first-run work such as downloading block lists is over).

| | Footprint median | Min | Max | RSS median | Processes | Where it goes (last run) |
|---|---|---|---|---|---|---|
| Wisp | **188 MB** | 176 | 188 | 167 MB | 6 | 2 WebContent (interface + page) 81 MB, GPU 74 MB, Wisp 22 MB, Networking 8 MB, audio helper 3 MB |
| Zen | 716 MB | 697 | 747 | 2,218 MB | 13 | GPU helper 250 MB, 10 content processes 246 MB, main 198 MB, video decoder 4 MB |
| Chrome | 239 MB | 234 | 240 | 608 MB | 8 | main 103 MB, 3 helpers (GPU, network, storage) 71 MB, 3 renderers 64 MB, crash handler 3 MB |

A first pass took the sample 30 s after load on a brand-new profile each run, and gave Wisp 197 MB, Chrome 242 MB and Zen 957 MB. Zen's main process was 619 MB then and 198 MB here: it was still doing first-run work. That pass is kept in the data file as `onetab_first_pass_superseded` and is not used.

Not like-for-like: Wisp's one tab is two web-content processes, because its own interface is a web page too. Zen starts a pool of ten content processes even for one tab.

#### Ten real sites

The same ten pages in every browser: Wikipedia (Web browser), GitHub (WebKit/WebKit), Hacker News, BBC News, The Guardian, Stack Overflow (rust tag), MDN JavaScript, The New York Times, Amazon, YouTube. Sampled 100 seconds after launch, then the idle minute below. Fresh profile each run; runs interleaved across browsers so changes in the live sites hit everyone alike.

| | Footprint median | Min | Max | RSS median | Processes |
|---|---|---|---|---|---|
| **Wisp, all ten loaded** (fair comparison) | 2,448 MB | 2,438 | 2,541 | 763 MB | 20 |
| Wisp, restored, only the active tab loaded | 281 MB | 180 | 282 | 185 MB | 6 |
| Zen | 2,218 MB | 2,136 | 2,268 | 5,614 MB | 27 |
| Chrome | **1,152 MB** | 1,095 | 1,280 | 2,122 MB | 25 |

Where it goes (last run):

| | Main process | Page processes | GPU | Network / other |
|---|---|---|---|---|
| Wisp, all loaded | 37 MB | 2,178 MB (14 WebContent) | 163 MB | 57 + 15 MB |
| Zen | 325 MB | 1,385 MB (23 plugin-container) | 422 MB | 4 MB |
| Chrome | 147 MB | 869 MB (16 renderers) | 125 MB (5 other helpers, GPU among them) | 12 MB |

Not like-for-like:

- **How Wisp's tabs were loaded.** Wisp only builds a restored tab's web view when the tab is first shown, so a ten-tab session file gives one loaded page. For the fair row, a local page opens the ten sites with `window.open`, one every four seconds; Wisp turns each into a new tab and shows it, so every site is loaded and then sent to the background. The last sites had about 60 s to settle instead of 100 s. Zen and Chrome were given the ten URLs on the command line and load them all at launch.
- **Blocking differs.** Wisp blocks 157 ad and tracker hosts, Zen blocks trackers (Enhanced Tracking Protection, Standard), Chrome blocks nothing by default. Pages therefore download different things in each browser.
- **User agents differ.** Wisp presents itself as Safari 26.5.2, so sites may serve it different pages from the ones Chrome and Zen get.
- **Wisp's 14 web-content processes for ten tabs** are one for the interface, one per site, and a few WebKit keeps for cross-site navigations; all counted.

#### Ten real sites with tab unloading

Measured after the report above, on the tab-unloading change (#33): the same ten sites, loaded the same way (`window.open` from a local page), in the release `Wisp.app` with the same process accounting. Unloading was set to 1 minute so the sample at 150 s falls after the nine background tabs became due. Other Wisp benchmarks ran on the machine at the same time, so treat single runs with some care.

| | Footprint, three runs | Page processes |
|---|---|---|
| Wisp, all ten loaded (unloading off) | 2,357 / 2,303 / 2,422 MB | 12-14 WebContent |
| Wisp, nine unloaded | **497 / 519 / 516 MB** | 2 WebContent |

Coming back to an unloaded tab, from selecting it to the page's `load` event (warm disk cache): Wikipedia 3.6 s, GitHub 0.80, Hacker News 0.28, BBC 0.40, The Guardian 0.21, Stack Overflow 0.68, MDN 0.16, The New York Times 1.15, Amazon 1.45, YouTube 0.99 s; median about 0.74 s. Pages draw before `load`. The default is 30 minutes, and tabs making sound, in a call, with typed text, private or pinned stay loaded (see `unload_due` and `keeps` in `src/main.rs`). The feature table below describes the code before this change.

### Idle cost

CPU of the whole tree over one minute with the ten tabs open and nothing happening, right after the memory sample.

| | CPU (% of one core), median | Min | Max | Wake-ups per second |
|---|---|---|---|---|
| Wisp, all ten loaded | 32.0 % | 31.1 | 32.4 | 453 |
| Wisp, restored, one loaded | 17.9 % | 0.2 | 20.7 | 238 |
| Zen | 31.8 % | 28.7 | 35.7 | 195 |
| Chrome | **13.0 %** | 11.7 | 15.5 | 319 |

With one simple page, right after the one-tab memory sample:

| | CPU (% of one core), median | Min | Max | Wake-ups per second |
|---|---|---|---|---|
| Wisp | 20.1 % | 17.0 | 20.4 | 194 |
| Zen | **0.02 %** | 0.02 | 0.07 | 0.7 |
| Chrome | 0.25 % | 0.22 | 0.31 | 4.0 |

**Why Wisp is busy at idle.** With one idle simple page, the CPU by process was: interface web content 6.5 %, WebKit GPU process 6.2 %, Wisp 4.0 %, the page itself 0.02 %. So the cost is Wisp's own interface, not the page. The cause is `#progress::after { animation: sweep 1.1s … infinite }` in `src/ui.html`, which keeps running while `#progress` is only made transparent. A/B test with a debug build, one simple tab, 30 s idle, the interface script `document.getElementById('progress').style.display = 'none'` injected through `WISP_JS`:

| | Wisp process | Whole tree |
|---|---|---|
| As shipped | 5.1 %, 5.7 %, 5.3 % | 19.3 % |
| Progress bar removed | 0.48 %, 0.53 %, 0.48 % | 1.5 % |

The one restored run with 0.2 % idle CPU is probably the window being fully covered by other windows, which lets WebKit stop painting it; that the idle cost depends on whether Wisp is visible is consistent with the cause above.

**Collapsed bar.** Release build, one simple tab, 60 s idle, two runs each: default layout 14.6 % and 12.8 %, bar collapsed 15.6 % and 14.0 % (whole tree). The difference is in the Wisp process (3.5 → 4.7 %, 3.2 → 4.4 %), which is the 25 ms cursor poll. Measured with a quick script (numbers in `diagnostics` in the data file), not with `bench.py`.

### Engine speed: Speedometer 3.1

The official Speedometer 3.1 (branch `release/3.1` of [WebKit/Speedometer](https://github.com/WebKit/Speedometer), the code behind browserbench.org/Speedometer3.1) served from the local bench server, started with `?startAutomatically=true`. A small script added to the page reads the final score and sends it back to the bench server, which is how the score is read without screen capture or browser automation. One full Speedometer run (10 iterations) per launch, three launches, a fresh window each time.

| | Score median | Min | Max | Runs (score ± Speedometer's own 95 % interval) |
|---|---|---|---|---|
| Wisp | 45.2 | 43.8 | 45.7 | 45.2 ± 3.3, 43.8 ± 2.2, 45.7 ± 3.1 |
| Zen | 46.0 | 40.0 | 47.4 | 40.0 ± 5.6, 46.0 ± 3.5, 47.4 ± 3.9 |
| Chrome | **59.8** | 59.1 | 60.6 | 59.1 ± 3.3, 59.8 ± 3.5, 60.6 ± 2.7 |

Not like-for-like: served from `127.0.0.1` rather than browserbench.org (no network in the timing either way). Windows were opened in the background; the page never saw itself hidden (`visibilitychange` count 0 in every run), but other work on the machine was going on. Wisp's interface was burning CPU throughout (the progress-bar bug above) which may cost it a little.

## Performance features

From each browser's official documentation, or for Wisp, from its code at `73f906f`. Where Zen has no documentation of its own, Firefox's documentation or Firefox's shipped default settings are used and marked *(Firefox)*: Zen is built on Firefox and keeps its defaults unless it says otherwise.

| Feature | Wisp | Zen | Chrome | Safari |
|---|---|---|---|---|
| **Tab sleeping / discarding** | None. Once a tab's web view is built it stays until the tab is closed. Hidden tabs are only `set_visible(false)`, leaving throttling to WebKit. | Firefox's low-memory tab unloading is on, but its trigger is disabled on macOS outside Nightly (`browser.lowMemoryResponseMask` 0) [1] *(Firefox)*; manual "Unload" in the tab menu is enabled by Zen [2]. A Zen issue reports no automatic unloading on macOS since Zen dropped its own unloader; maintainers say they keep Firefox's defaults [3] (an issue, not documentation). | Memory Saver deactivates unused tabs; three levels; sites can be exempted [4][5]. | No discarding documented. Background tabs are throttled (timers, `requestAnimationFrame`) and take part in App Nap [6]. |
| **Memory / energy saver** | None. | No official source found. | Energy Saver (on battery or low battery) limits background activity and frame rates [4][5]. | No official source found for macOS. |
| **Lazy session restore** | Yes: restored tabs are not loaded until first shown (`App::select`, `src/main.rs`). Their favicons are still fetched at start. | Yes: `browser.sessionstore.restore_on_demand` true [1] *(Firefox)*; Zen also makes pinned tabs lazy [2]. Zen restores the previous session by default (`browser.startup.page` 3 in Zen's shipped prefs). | No official source found. | No official source found. |
| **Per-tab resource display** | None. | `about:processes` (Shift+Esc) with memory and CPU per tab [7] *(Firefox)*. | Task Manager [8]; memory use on tab hover cards [4]. | None in Safari; Web Inspector Timelines for a page's CPU [6]. |
| **Preloading / prerendering** | None of its own. | Link prefetch and speculative connections from the address bar on by default [9] *(Firefox)*. No prerender documented. | "Preload pages" setting; address-bar prerender and Speculation Rules [8][10]. | "Preload Top Hit in the background" from the search field [11]. |
| **Hardware video decoding** | Whatever WebKit does (same as Safari). | VideoToolbox for H.264, HEVC, VP9 and AV1 on Apple silicon (from Firefox's source) [12] *(Firefox)*. | VideoToolbox for H.264, VP9, AV1; HEVC with hardware support [13][14]. | VP9, H.264, HEVC and AV1 in hardware on supported Macs [15]. |
| **Network-level content blocking** | Yes: 157 built-in ad/tracker hosts plus user-added ones, compiled into a WebKit content rule list (macOS) or a WebView2 request filter (Windows). No cosmetic filtering, no filter-list syntax (`src/adblock.rs`). Popups to blocked hosts are dropped. | Enhanced Tracking Protection: Standard, Strict, Custom [16]. | No built-in ad blocker beyond the "Intrusive ads" setting [17]; Safe Browsing [18]; extensions can block with `declarativeNetRequest` [19]. | Intelligent Tracking Prevention blocks third-party cookies [20]; content-blocker extensions. |
| **Startup optimisations** | Tiny binary (LTO, one codegen unit, stripped, `panic = abort`); no engine to load beyond the system's; first update check 5 s after start. The first tab waits for the ad-block rules to compile. | Release builds use optimisation, cross-language LTO, Rust SIMD and on macOS profile-guided optimisation tuned for Apple M1 [21]; release notes claim faster starts with many tabs (1.23b) [22]. | Profile-guided optimisation since Chrome 85 [23]. | No official source found. |

Sources (all opened 16 November 2026):

1. Firefox default prefs, `browser/app/profile/firefox.js`: https://raw.githubusercontent.com/mozilla-firefox/firefox/main/browser/app/profile/firefox.js (the copy shipped in Zen 1.24.1b's `omni.ja` has the same values)
2. Zen prefs: https://raw.githubusercontent.com/zen-browser/desktop/dev/prefs/firefox/browser.yaml
3. https://github.com/zen-browser/desktop/issues/15619
4. https://support.google.com/chrome/answer/12929150
5. https://developer.chrome.com/blog/memory-and-energy-saver-mode
6. https://webkit.org/blog/8970/how-web-content-can-affect-power-usage/
7. https://support.mozilla.org/en-US/kb/task-manager-tabs-or-extensions-are-slowing-firefox (page behind a bot check; content confirmed from search result text only)
8. https://support.google.com/chrome/answer/1385029
9. Firefox `modules/libpref/init/all.js`: https://raw.githubusercontent.com/mozilla-firefox/firefox/main/modules/libpref/init/all.js, and [1]
10. https://developer.chrome.com/docs/web-platform/prerender-pages
11. https://support.apple.com/guide/safari/search-sfria1042d31/mac
12. https://raw.githubusercontent.com/mozilla-firefox/firefox/main/dom/media/platforms/apple/AppleDecoderModule.cpp
13. https://chromium.googlesource.com/chromium/src/+/main/media/gpu/mac/video_toolbox_video_decoder.cc
14. https://www.chromium.org/audio-video/
15. https://webkit.org/blog/15063/webkit-features-in-safari-17-4/
16. https://docs.zen-browser.app/security
17. https://support.google.com/chrome/answer/2765944
18. https://support.google.com/chrome/answer/9890866
19. https://developer.chrome.com/docs/extensions/reference/api/declarativeNetRequest
20. https://webkit.org/tracking-prevention/
21. https://raw.githubusercontent.com/zen-browser/desktop/dev/configs/common/mozconfig and https://raw.githubusercontent.com/zen-browser/desktop/dev/configs/macos/mozconfig
22. https://zen-browser.app/release-notes/
23. https://blog.chromium.org/2020/08/chrome-just-got-faster-with-profile.html

Sources 1, 2, 9, 12, 13 and 21 are the projects' current main branches, not pinned to the exact versions measured.

## Running it again

```bash
cargo build --release && cargo packager --release --formats app
git clone --depth 1 --branch release/3.1 https://github.com/WebKit/Speedometer.git /tmp/speedometer
python3 bench/bench.py size
python3 bench/bench.py coldstart
python3 bench/bench.py onetab
python3 bench/bench.py tentabs
python3 bench/bench.py speedometer --src /tmp/speedometer
```

Each command merges its results into `docs/performance-data.json`. `--browsers wisp,zen` limits the set, `--runs N` changes the count. Close nothing beforehand: the script starts and stops only its own instances. Expect about an hour in all.
