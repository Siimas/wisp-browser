//! Ad and tracker blocking with the filter lists popular blockers use (EasyList and
//! friends), done inside the engine's network layer where the platform allows it:
//!
//! - macOS: each list is converted (`filters.rs`) into WebKit content rules and
//!   compiled once per list version into WebKit's rule store, then looked up by name at
//!   launch. Blocking and element hiding then happen inside WebKit, at no cost here.
//! - Windows: WebView2 has no rule engine, so its request filter asks a small matcher
//!   (`filters::matcher`) about each request. Element hiding is not done there.
//!
//! Lists are downloaded from their publishers into the config folder and refreshed in
//! the background; the built-in host list (`blocklist.txt`) is used until one arrives.

use crate::{config, config::Adblock, filters};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    fs,
    hash::{DefaultHasher, Hash, Hasher},
    io::Read,
    path::PathBuf,
    sync::{Mutex, RwLock, mpsc},
    time::{Duration, Instant, SystemTime},
};

pub struct List {
    pub id: &'static str,
    pub name: &'static str,
    pub about: &'static str,
    url: &'static str,
}

/// The lists offered in Settings, all fetched from their publishers over HTTPS.
/// None of them is copied into this repository (they are GPL / CC BY-SA licensed).
pub const LISTS: [List; 5] = [
    List {
        id: "easylist",
        name: "EasyList",
        about: "Ads",
        url: "https://easylist.to/easylist/easylist.txt",
    },
    List {
        id: "easyprivacy",
        name: "EasyPrivacy",
        about: "Trackers",
        url: "https://easylist.to/easylist/easyprivacy.txt",
    },
    List {
        id: "cookies",
        name: "EasyList Cookie List",
        about: "Cookie notices",
        url: "https://secure.fanboy.co.nz/fanboy-cookiemonster.txt",
    },
    List {
        id: "annoyances",
        name: "Fanboy's Annoyance List",
        about: "Pop-ups, social widgets, cookie notices",
        url: "https://easylist.to/easylist/fanboy-annoyance.txt",
    },
    List {
        id: "peterlowe",
        name: "Peter Lowe's List",
        about: "Ad and tracker servers",
        url: "https://pgl.yoyo.org/adservers/serverlist.php?hostformat=adblockplus&showintro=0&mimetype=plaintext",
    },
];

/// Lists are fetched again once their copy is this old (EasyList says "Expires: 4 days").
const REFRESH: Duration = Duration::from_secs(4 * 24 * 3600);
/// The largest list today is about 2 MB.
const MAX_BYTES: u64 = 16 << 20;
/// Bump when the conversion changes, so stored compiled lists are rebuilt.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const FORMAT: u32 = 1;

fn folder() -> PathBuf {
    config::dir().join("filters")
}

fn list_path(id: &str) -> PathBuf {
    folder().join(format!("{id}.txt"))
}

/// Touched whenever a list is checked, changed or not.
fn checked_path(id: &str) -> PathBuf {
    folder().join(format!("{id}.checked"))
}

fn modified(path: &PathBuf) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// When the list was last fetched or confirmed current.
fn checked(id: &str) -> Option<SystemTime> {
    modified(&list_path(id)).max(modified(&checked_path(id)))
}

fn valid_host(h: &str) -> bool {
    h.contains('.')
        && h.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

fn clean_hosts<'a>(lines: impl Iterator<Item = &'a str>) -> Vec<String> {
    lines
        .map(|l| l.trim().to_ascii_lowercase())
        .filter(|l| !l.starts_with('#') && valid_host(l))
        .collect()
}

fn builtin_hosts() -> Vec<String> {
    clean_hosts(include_str!("blocklist.txt").lines())
}

// ---------------------------------------------------------------- shared state

/// Built-in and user-added hosts, for the popup check. `None` = blocking off.
static HOSTS: RwLock<Option<HashSet<String>>> = RwLock::new(None);
/// Blocking on at all, and the sites it is turned off for.
static SITES: RwLock<(bool, Vec<String>)> = RwLock::new((false, Vec::new()));
/// The settings the rules were last built from.
static CFG: Mutex<Option<Adblock>> = Mutex::new(None);
/// Tells the app that rules changed or a list was fetched (it re-attaches and redraws).
#[allow(clippy::type_complexity)]
static NOTIFY: Mutex<Option<Box<dyn Fn() + Send>>> = Mutex::new(None);
/// Wakes the list updater; `true` = fetch every list now ("Update now").
static WAKE: Mutex<Option<mpsc::Sender<bool>>> = Mutex::new(None);
/// When a fetch last failed, so a computer that is offline is not asked again and again.
static LAST_FAILED: Mutex<Option<Instant>> = Mutex::new(None);
/// The updater is working / the last problem it had, for Settings.
static STATUS: Mutex<(bool, String)> = Mutex::new((false, String::new()));

fn notify() {
    if let Some(f) = NOTIFY.lock().unwrap().as_ref() {
        f();
    }
}

fn enabled_lists(cfg: &Adblock) -> Vec<&'static List> {
    let on = |l: &&List| cfg.enabled && cfg.lists.iter().any(|id| id == l.id);
    LISTS.iter().filter(on).collect()
}

/// A site as stored for the per-site switch: lowercase host without `www.`.
pub fn site_key(host: &str) -> String {
    let h = host.trim().to_ascii_lowercase();
    h.strip_prefix("www.").unwrap_or(&h).to_string()
}

fn listed(set: &[String], host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    let mut h = host.as_str();
    loop {
        if set.iter().any(|s| s == h) {
            return true;
        }
        match h.split_once('.') {
            Some((_, parent)) => h = parent,
            None => return false,
        }
    }
}

/// Whether pages at this address get blocking (on, and not turned off for its site).
pub fn enabled_for(url: &str) -> bool {
    let sites = SITES.read().unwrap();
    sites.0 && !host_of(url).is_some_and(|h| listed(&sites.1, h))
}

/// Take new settings. Cheap when only the per-site switches changed; rebuilds rules
/// when the lists or extra hosts did. `notify` runs (on any thread) when rules are
/// ready or the list status changed; the first call is when tabs may start loading.
pub fn configure(cfg: &Adblock, notify_app: impl Fn() + Send + 'static) {
    *NOTIFY.lock().unwrap() = Some(Box::new(notify_app));
    set_switches(cfg);
    let rules_cfg = Adblock {
        off_sites: Vec::new(),
        ..cfg.clone()
    };
    let changed = CFG.lock().unwrap().replace(rules_cfg.clone()) != Some(rules_cfg.clone());
    start_updater();
    if enabled_lists(cfg).iter().any(|l| !list_path(l.id).exists()) {
        wake(false);
    }
    if changed {
        platform::rebuild(&rules_cfg);
    } else {
        notify();
    }
}

/// The on/off state per site and the popup host list.
fn set_switches(cfg: &Adblock) {
    let off: Vec<String> = cfg.off_sites.iter().map(|s| site_key(s)).collect();
    *SITES.write().unwrap() = (cfg.enabled, off);
    *HOSTS.write().unwrap() = cfg.enabled.then(|| {
        let extra = clean_hosts(cfg.extra_hosts.iter().map(String::as_str));
        builtin_hosts().into_iter().chain(extra).collect()
    });
}

/// A set of rules compiled as one unit: one version of a downloaded list, the built-in
/// hosts, or the user's extra hosts.
#[derive(Clone)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
struct Source {
    /// Names the compiled rules: changes whenever their content would.
    name: String,
    /// The list it belongs to, for the records kept per list.
    list_id: Option<&'static str>,
    /// The file to build it from. `None` with no hosts: only look it up (a previous
    /// version, kept because the current one would not compile).
    list: Option<PathBuf>,
    hosts: Vec<String>,
    /// How many parts the stored version has, when known.
    parts: Option<usize>,
}

fn hash(x: impl Hash) -> u64 {
    let mut h = DefaultHasher::new();
    x.hash(&mut h);
    h.finish()
}

/// `<id>.compiled`: "name parts" of the last version of a list that compiled fully.
/// `<id>.failed`: the name of a version that would not compile.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn record_path(id: &str, kind: &str) -> PathBuf {
    folder().join(format!("{id}.{kind}"))
}

fn record(id: &str) -> Option<(String, usize)> {
    let text = fs::read_to_string(record_path(id, "compiled")).ok()?;
    let (name, parts) = text.trim().split_once(' ')?;
    Some((name.to_string(), parts.parse().ok()?))
}

fn sources(cfg: &Adblock) -> Vec<Source> {
    if !cfg.enabled {
        return Vec::new();
    }
    let mut out = Vec::new();
    for l in enabled_lists(cfg) {
        let path = list_path(l.id);
        let Ok(meta) = fs::metadata(&path) else {
            continue;
        };
        let stamp = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok());
        let name = format!("wisp-{}-{:x}-{FORMAT}", l.id, hash((meta.len(), stamp)));
        let failed = fs::read_to_string(record_path(l.id, "failed")).ok();
        let source = match record(l.id) {
            // This version would not compile: keep using the last one that did.
            Some((good, parts)) if failed.as_deref() == Some(&name) && good != name => Source {
                name: good,
                list_id: Some(l.id),
                list: None,
                hosts: Vec::new(),
                parts: Some(parts),
            },
            rec => Source {
                parts: rec.filter(|(r, _)| *r == name).map(|(_, n)| n),
                name,
                list_id: Some(l.id),
                list: Some(path),
                hosts: Vec::new(),
            },
        };
        out.push(source);
    }
    let hosts_source = |kind: &str, hosts: Vec<String>| Source {
        name: format!("wisp-{kind}-{:x}-{FORMAT}", hash(&hosts)),
        list_id: None,
        list: None,
        hosts,
        parts: Some(1),
    };
    // The built-in hosts are always in: a few hundred rules, and they keep blocking on
    // while a list is being converted, or when one cannot be used.
    out.push(hosts_source("builtin", builtin_hosts()));
    let extra = clean_hosts(cfg.extra_hosts.iter().map(String::as_str));
    if !extra.is_empty() {
        out.push(hosts_source("extra", extra));
    }
    out
}

impl Source {
    fn rules(&self) -> Vec<filters::Rule> {
        match &self.list {
            Some(path) => fs::read_to_string(path)
                .map(|text| text.lines().filter_map(filters::parse).collect())
                .unwrap_or_default(),
            None => self
                .hosts
                .iter()
                .filter_map(|h| filters::parse(&format!("||{h}^")))
                .collect(),
        }
    }
}

// ---------------------------------------------------------------- per-page switch

thread_local! {
    /// What each web view has attached: (rules version, blocking on). Keyed by the
    /// engine object's address; a new view is cleared first (`attach_new`).
    /// ponytail: entries of closed views stay (a few bytes each).
    static ATTACHED: RefCell<HashMap<usize, (u64, bool)>> = RefCell::new(HashMap::new());
}

/// For a web view just made: forget whatever an earlier view at the same address had.
pub fn attach_new(webview: &wry::WebView, url: &str) {
    let key = platform::key(webview);
    ATTACHED.with_borrow_mut(|m| m.remove(&key));
    attach(webview, url);
}

/// Give this web view the current rules, on or off for the site at `url` (the UI
/// views pass ""). Called when a view is made, when its page commits, and on
/// `notify`; the platform also calls `switch` just before each page navigation, so
/// a page always starts with the right rules. Returns true when this changed
/// blocking for the page already shown (which then needs loading again).
pub fn attach(webview: &wry::WebView, url: &str) -> bool {
    switch(platform::key(webview), url, |on, new| {
        platform::apply(webview, platform::key(webview), on, new)
    })
}

/// Set the view `key` to the right state for `url`; `apply(on, new view)` runs when
/// the state or the rules changed. Returns whether blocking was switched.
fn switch(key: usize, url: &str, apply: impl FnOnce(bool, bool)) -> bool {
    let on = enabled_for(url);
    let now = (platform::version(), on);
    let before = ATTACHED.with_borrow_mut(|m| m.insert(key, now));
    if before != Some(now) {
        apply(on, before.is_none());
    }
    before.is_some_and(|(_, was)| was != on)
}

pub fn host_of(url: &str) -> Option<&str> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '\\', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    host.split(':').next().filter(|h| !h.is_empty())
}

/// True when the URL's host, or any parent domain of it, is on the built-in or
/// user-added list. Used for popups, which never reach the request filters.
pub fn is_blocked(url: &str) -> bool {
    let guard = HOSTS.read().unwrap();
    let (Some(set), Some(host)) = (guard.as_ref(), host_of(url)) else {
        return false;
    };
    let host = host.to_ascii_lowercase();
    let mut h = host.as_str();
    loop {
        if set.contains(h) {
            return true;
        }
        match h.split_once('.') {
            Some((_, parent)) => h = parent,
            None => return false,
        }
    }
}

// ---------------------------------------------------------------- list updates

/// Fetch every list now (Settings > Update now).
pub fn update_now() {
    wake(true);
}

fn wake(all: bool) {
    if let Some(tx) = WAKE.lock().unwrap().as_ref() {
        let _ = tx.send(all);
    }
}

/// One background thread, asleep between checks: shortly after launch, then hourly,
/// it fetches each enabled list whose copy is older than `REFRESH`. Private windows
/// (a second process on the same folder) leave this to the main window.
fn start_updater() {
    let mut wake = WAKE.lock().unwrap();
    if wake.is_some() || std::env::args().any(|a| a == "--private") {
        return;
    }
    let (tx, rx) = mpsc::channel();
    *wake = Some(tx);
    std::thread::spawn(move || {
        let mut wait = Duration::from_secs(10); // let the first pages load first
        loop {
            let all = match rx.recv_timeout(wait) {
                Ok(all) => all,
                Err(mpsc::RecvTimeoutError::Timeout) => false,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            };
            wait = Duration::from_secs(3600);
            update(all);
        }
    });
}

fn update(all: bool) {
    let Some(cfg) = CFG.lock().unwrap().clone() else {
        return;
    };
    let due =
        |l: &&List| all || checked(l.id).is_none_or(|t| t.elapsed().unwrap_or_default() > REFRESH);
    let lists: Vec<&List> = enabled_lists(&cfg).into_iter().filter(due).collect();
    // After a failed attempt (offline, say), wait a while before the next one, however
    // often settings are saved; "Update now" still tries at once.
    let resting = LAST_FAILED
        .lock()
        .unwrap()
        .is_some_and(|t| t.elapsed() < Duration::from_secs(15 * 60));
    if lists.is_empty() || resting && !all {
        return;
    }
    STATUS.lock().unwrap().0 = true;
    notify();
    let (mut changed, mut problems) = (false, Vec::new());
    for l in lists {
        match fetch(l) {
            Ok(c) => changed |= c,
            Err(e) => problems.push(format!("{}: {e}", l.name)),
        }
    }
    *LAST_FAILED.lock().unwrap() = (!problems.is_empty()).then(Instant::now);
    *STATUS.lock().unwrap() = (false, problems.join(". "));
    if changed {
        platform::lists_changed();
    }
    notify();
}

/// Download one list. Ok(true) when the stored copy changed. Anything that does not
/// look like a good list is refused and the copy we have stays in use.
fn fetch(list: &List) -> Result<bool, String> {
    // No cookies, no referrer, no identifier: the publisher sees an address asking
    // for a public file. Written like the updater's requests, so no new code is built in.
    let response = reqwest::blocking::Client::new()
        .get(list.url)
        .header(reqwest::header::USER_AGENT, "Wisp")
        .timeout(Duration::from_secs(60))
        .send()
        .map_err(|e| e.to_string())?;
    if response.url().scheme() != "https" {
        return Err("the list was redirected to an unencrypted address".into());
    }
    if !response.status().is_success() {
        return Err(format!("the server answered {}", response.status()));
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let old = fs::read_to_string(list_path(list.id)).ok();
    let text = check(&bytes, old.as_deref())?;
    fs::create_dir_all(folder()).map_err(|e| e.to_string())?;
    let changed = old.as_deref() != Some(text);
    if changed {
        let part = folder().join(format!("{}.part", list.id));
        fs::write(&part, text)
            .and_then(|_| fs::rename(&part, list_path(list.id)))
            .map_err(|e| e.to_string())?;
    }
    fs::write(checked_path(list.id), "").map_err(|e| e.to_string())?;
    Ok(changed)
}

fn rule_lines(text: &str) -> usize {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with(['!', '[']))
        .count()
}

/// Is this download a filter list we should switch to? Not when it is too big, not
/// text, not in the list format (an error page, say), or less than half the size of
/// the copy we have (cut short).
fn check<'a>(bytes: &'a [u8], old: Option<&str>) -> Result<&'a str, String> {
    if bytes.len() as u64 > MAX_BYTES {
        return Err("the download is too large".into());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "the download is not text")?;
    let first = text.lines().map(str::trim).find(|l| !l.is_empty());
    if !first.is_some_and(|l| l.starts_with("[Adblock") || l.starts_with('!')) {
        return Err("the download is not a filter list".into());
    }
    let rules = rule_lines(text);
    if rules < 100 || rules < old.map_or(0, rule_lines) / 2 {
        return Err("the download is much shorter than expected".into());
    }
    Ok(text)
}

/// For Settings: each list with when it was last checked, and the updater's state.
pub fn status() -> Value {
    let lists: Vec<Value> = LISTS
        .iter()
        .map(|l| {
            let at = checked(l.id)
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_secs());
            json!({"id": l.id, "name": l.name, "about": l.about, "checked": at})
        })
        .collect();
    let (busy, problem) = STATUS.lock().unwrap().clone();
    json!({"lists": lists, "busy": busy, "problem": problem})
}

// ---------------------------------------------------------------- macOS

#[cfg(target_os = "macos")]
mod platform {
    use super::{Source, notify, record_path, sources, switch};
    use crate::{config::Adblock, filters};
    use block2::RcBlock;
    use dispatch2::DispatchQueue;
    use objc2::{
        MainThreadMarker, msg_send,
        rc::Retained,
        runtime::{AnyObject, Imp, Sel},
        sel,
    };
    use objc2_foundation::{NSArray, NSError, NSString};
    use objc2_web_kit::{WKContentRuleList, WKContentRuleListStore, WKUserContentController};
    use std::{
        cell::{Cell, RefCell},
        collections::{BTreeMap, HashMap, HashSet},
        fs,
        rc::Rc,
        sync::OnceLock,
        time::Instant,
    };
    use wry::{WebView, WebViewExtMacOS};

    /// Rules per compiled part. WebKit takes up to 150,000 on current systems; smaller
    /// parts compile faster and stay under the 50,000 that older versions allowed.
    const PART: usize = 50_000;
    /// Parts kept per list: 200,000 rules, over three times EasyList's.
    const MAX_PARTS: usize = 4;

    /// How a wanted source stands.
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        /// Parts still to come (looked up or compiled).
        Waiting(usize),
        Ready,
        Failed,
    }

    thread_local! {
        /// Compiled parts in use, by name (`<source>.<part>`).
        static RULES: RefCell<BTreeMap<String, Retained<WKContentRuleList>>> = RefCell::default();
        /// The sources wanted now.
        static WANT: RefCell<HashMap<String, State>> = RefCell::default();
        /// Sources whose every part is in `RULES`.
        static COMPLETE: RefCell<HashSet<String>> = RefCell::default();
        /// Lookups (and small compiles) not answered yet; the first `notify`, when tabs
        /// start, waits for them but not for big lists being compiled.
        static PENDING: Cell<usize> = const { Cell::new(0) };
        static VERSION: Cell<u64> = const { Cell::new(0) };
    }

    fn store() -> Option<Retained<WKContentRuleListStore>> {
        let mtm = MainThreadMarker::new().expect("adblock runs on the main thread");
        // SAFETY: main thread.
        unsafe { WKContentRuleListStore::defaultStore(mtm) }
    }

    fn source_of(part: &str) -> &str {
        part.rsplit_once('.').map_or(part, |(s, _)| s)
    }

    /// "wisp-easylist-<version>-<format>" -> "easylist".
    fn list_of(name: &str) -> &str {
        name.split('-').nth(1).unwrap_or("")
    }

    fn state(name: &str) -> Option<State> {
        WANT.with_borrow(|w| w.get(name).copied())
    }

    fn set_state(name: &str, st: State) {
        WANT.with_borrow_mut(|w| {
            if let Some(s) = w.get_mut(name) {
                *s = st;
            }
        });
    }

    pub fn version() -> u64 {
        VERSION.get()
    }

    pub fn key(webview: &WebView) -> usize {
        Retained::as_ptr(&webview.manager()) as usize
    }

    pub fn apply(webview: &WebView, _key: usize, on: bool, new: bool) {
        if new {
            hook_navigation(webview);
        }
        apply_to(&webview.manager(), on);
    }

    /// Put a view's rule lists in place: all of them, or none.
    fn apply_to(manager: &WKUserContentController, on: bool) {
        // SAFETY: main-thread calls on a live WKUserContentController.
        unsafe {
            manager.removeAllContentRuleLists();
            if on {
                RULES.with_borrow(|r| r.values().for_each(|l| manager.addContentRuleList(l)));
            }
        }
    }

    // ------------------------------------------------ the per-site switch, before loading

    static DECIDE: OnceLock<Imp> = OnceLock::new();

    /// Runs in place of the web view delegate's `decidePolicyForNavigationAction`: for
    /// a page (main-frame) navigation, the view gets the rules for the site it is about
    /// to load, before the request goes out; then the original method decides as before.
    /// So a site switched off never loads with blocking on, or the reverse, and nothing
    /// has to be loaded twice. Frames, which belong to their page, leave it alone.
    unsafe extern "C-unwind" fn decide(
        this: &AnyObject,
        cmd: Sel,
        view: *mut AnyObject,
        action: *mut AnyObject,
        handler: *mut AnyObject,
    ) {
        // SAFETY: WebKit passes a live web view and navigation action; main thread.
        unsafe {
            let frame: *mut AnyObject = msg_send![action, targetFrame];
            let main = !frame.is_null() && {
                let m: bool = msg_send![frame, isMainFrame];
                m
            };
            if main {
                let request: *mut AnyObject = msg_send![action, request];
                let address: *mut AnyObject = msg_send![request, URL];
                let text: *mut NSString = if address.is_null() {
                    std::ptr::null_mut()
                } else {
                    msg_send![address, absoluteString]
                };
                let config: *mut AnyObject = msg_send![view, configuration];
                let manager: *mut WKUserContentController =
                    msg_send![config, userContentController];
                if let (Some(url), Some(manager)) = (text.as_ref(), manager.as_ref()) {
                    let key = manager as *const WKUserContentController as usize;
                    switch(key, &url.to_string(), |on, _| apply_to(manager, on));
                }
            }
            if let Some(original) = DECIDE.get() {
                let original: unsafe extern "C-unwind" fn(
                    &AnyObject,
                    Sel,
                    *mut AnyObject,
                    *mut AnyObject,
                    *mut AnyObject,
                ) = std::mem::transmute(*original);
                original(this, cmd, view, action, handler);
            }
        }
    }

    /// Put `decide` in front of the delegate class's own method (once; every web view
    /// gets the same delegate class from wry).
    fn hook_navigation(webview: &WebView) {
        if DECIDE.get().is_some() {
            return;
        }
        let view = webview.webview();
        // SAFETY: main-thread messages to the live view; swaps an existing method of the
        // delegate class for one with the same signature (void; self, _cmd, view,
        // action, block), keeping the original to call.
        unsafe {
            let delegate: *mut AnyObject = msg_send![&*view, navigationDelegate];
            let Some(delegate) = delegate.as_ref() else {
                return;
            };
            let sel = sel!(webView:decidePolicyForNavigationAction:decisionHandler:);
            let method = objc2::ffi::class_getInstanceMethod(delegate.class(), sel);
            if method.is_null() {
                return;
            }
            let ours: Imp = std::mem::transmute(
                decide
                    as unsafe extern "C-unwind" fn(
                        &AnyObject,
                        Sel,
                        *mut AnyObject,
                        *mut AnyObject,
                        *mut AnyObject,
                    ),
            );
            if let Some(original) = objc2::ffi::method_setImplementation(method, ours) {
                let _ = DECIDE.set(original);
            }
        }
    }

    // ------------------------------------------------ building the rules

    /// Called from the updater thread.
    pub fn lists_changed() {
        DispatchQueue::main().exec_async(|| {
            if let Some(cfg) = super::CFG.lock().unwrap().clone() {
                rebuild(&cfg);
            }
        });
    }

    /// Bring the compiled rules in line with the settings. Rules in use stay; stored
    /// versions are looked up; a list version never compiled is converted (on a worker
    /// thread) and compiled. Nothing in use is dropped, and nothing stored is deleted,
    /// until every wanted source is ready; a list version that will not compile is
    /// recorded, and the list's previous version stays in use.
    pub fn rebuild(cfg: &Adblock) {
        let sources = sources(cfg);
        WANT.set(
            sources
                .iter()
                .map(|s| {
                    let done = COMPLETE.with_borrow(|c| c.contains(&s.name));
                    (
                        s.name.clone(),
                        if done {
                            State::Ready
                        } else {
                            State::Waiting(usize::MAX)
                        },
                    )
                })
                .collect(),
        );
        hold(1);
        for s in sources {
            if state(&s.name) == Some(State::Ready) {
                continue;
            }
            match s.parts {
                Some(n) => look_up(s, n.min(MAX_PARTS)),
                None => build(s),
            }
        }
        release(1);
    }

    fn hold(n: usize) {
        PENDING.set(PENDING.get() + n);
    }

    fn release(n: usize) {
        PENDING.set(PENDING.get().saturating_sub(n));
        settle();
    }

    /// Look up the `n` stored parts of a source; if any is missing, build it again.
    fn look_up(source: Source, n: usize) {
        let Some(store) = store() else { return };
        set_state(&source.name, State::Waiting(n));
        let found = Rc::new(Cell::new((0, 0)));
        for i in 0..n {
            let id = format!("{}.{i}", source.name);
            let (found, source) = (found.clone(), source.clone());
            hold(1);
            let name = NSString::from_str(&id);
            let handler = RcBlock::new(move |list: *mut WKContentRuleList, _: *mut NSError| {
                // SAFETY: WebKit hands us a valid rule list or null.
                let list = unsafe { Retained::retain(list) };
                let (answers, hits) = found.get();
                let (answers, hits) = (answers + 1, hits + list.is_some() as usize);
                found.set((answers, hits));
                if let Some(list) = list
                    && state(&source.name).is_some()
                {
                    RULES.with_borrow_mut(|r| r.insert(id.clone(), list));
                }
                if answers == n {
                    if hits == n {
                        ready(&source.name);
                    } else if source.list.is_some() || !source.hosts.is_empty() {
                        build(source.clone()); // stored by another WebKit version, or cleared
                    } else {
                        set_state(&source.name, State::Failed);
                    }
                }
                release(1);
            });
            // SAFETY: main thread, live store; the block lives until WebKit calls it.
            unsafe {
                store.lookUpContentRuleListForIdentifier_completionHandler(
                    Some(&name),
                    Some(&handler),
                )
            };
        }
    }

    fn ready(name: &str) {
        if state(name).is_some() {
            set_state(name, State::Ready);
            COMPLETE.with_borrow_mut(|c| c.insert(name.to_string()));
            VERSION.set(VERSION.get() + 1);
        }
    }

    /// Convert a source on a worker thread, then compile its parts here.
    fn build(s: Source) {
        let small = s.list.is_none();
        if small {
            hold(1);
        }
        std::thread::spawn(move || {
            let started = Instant::now();
            let mut parts = filters::webkit::to_webkit(s.rules().into_iter(), PART);
            parts.truncate(MAX_PARTS);
            let took = started.elapsed();
            DispatchQueue::main().exec_async(move || {
                eprintln!(
                    "wisp: {} converted in {took:?} ({} parts)",
                    s.name,
                    parts.len()
                );
                if state(&s.name).is_some() {
                    compile_all(s, parts, small);
                }
                if small { release(1) } else { settle() }
            });
        });
    }

    fn compile_all(s: Source, parts: Vec<String>, small: bool) {
        let Some(store) = store() else { return };
        let n = parts.len();
        if n == 0 {
            return ready(&s.name);
        }
        set_state(&s.name, State::Waiting(n));
        let failed = Rc::new(Cell::new(false));
        let left = Rc::new(Cell::new(n));
        if small {
            hold(n);
        }
        for (i, json) in parts.into_iter().enumerate() {
            let id = format!("{}.{i}", s.name);
            let (s, failed, left) = (s.clone(), failed.clone(), left.clone());
            let (started, size) = (Instant::now(), json.len());
            let name = NSString::from_str(&id);
            let handler = RcBlock::new(move |list: *mut WKContentRuleList, err: *mut NSError| {
                // SAFETY: WebKit hands us a valid rule list, or null and an error.
                match unsafe { Retained::retain(list) } {
                    Some(list) => {
                        eprintln!(
                            "wisp: compiled {id} ({} KB) in {:?}",
                            size / 1024,
                            started.elapsed()
                        );
                        if state(&s.name).is_some() {
                            RULES.with_borrow_mut(|r| r.insert(id.clone(), list));
                        }
                    }
                    None => {
                        let why =
                            unsafe { err.as_ref() }.map(|e| e.localizedDescription().to_string());
                        eprintln!("wisp: ad block rules {id} failed to compile: {why:?}");
                        failed.set(true);
                    }
                }
                left.set(left.get() - 1);
                if left.get() == 0 {
                    finished(&s, n, failed.get());
                }
                if small { release(1) } else { settle() }
            });
            // SAFETY: as above.
            unsafe {
                store.compileContentRuleListForIdentifier_encodedContentRuleList_completionHandler(
                    Some(&name),
                    Some(&NSString::from_str(&json)),
                    Some(&handler),
                )
            };
        }
    }

    /// Every part of a build has answered: record the result for the next launch.
    fn finished(s: &Source, parts: usize, failed: bool) {
        let record = |kind: &str, text: String| {
            if let Some(id) = s.list_id {
                let _ = fs::write(record_path(id, kind), text);
            }
        };
        if !failed {
            record("compiled", format!("{} {parts}", s.name));
            if let Some(id) = s.list_id {
                let _ = fs::remove_file(record_path(id, "failed"));
            }
            return ready(&s.name);
        }
        // Not used half-built: drop its parts and keep the list's last good version.
        record("failed", s.name.clone());
        RULES.with_borrow_mut(|r| r.retain(|k, _| source_of(k) != s.name));
        set_state(&s.name, State::Failed);
        if let (Some(id), Some((good, n))) = (s.list_id, s.list_id.and_then(super::record)) {
            let loaded = COMPLETE.with_borrow(|c| c.contains(&good));
            if good != s.name && !loaded {
                WANT.with_borrow_mut(|w| w.insert(good.clone(), State::Waiting(n)));
                look_up(
                    Source {
                        name: good,
                        list_id: Some(id),
                        list: None,
                        hosts: Vec::new(),
                        parts: Some(n),
                    },
                    n,
                );
            }
        }
    }

    /// When nothing is waiting: drop rules no longer wanted (keeping a list's previous
    /// version while its new one failed), delete stored versions no longer used, and
    /// tell the app. While only big lists are compiling, just tell the app.
    fn settle() {
        if PENDING.get() > 0 {
            return;
        }
        let done = WANT.with_borrow(|w| w.values().all(|s| !matches!(s, State::Waiting(_))));
        if done {
            let failed: HashSet<String> = WANT.with_borrow(|w| {
                w.iter()
                    .filter(|(_, s)| **s == State::Failed)
                    .map(|(k, _)| list_of(k).to_string())
                    .collect()
            });
            let keep = |source: &str| {
                state(source) == Some(State::Ready)
                    || failed.contains(list_of(source))
                        && COMPLETE.with_borrow(|c| c.contains(source))
            };
            let before = RULES.with_borrow(|r| r.len());
            RULES.with_borrow_mut(|r| r.retain(|k, _| keep(source_of(k))));
            COMPLETE.with_borrow_mut(|c| c.retain(|s| keep(s)));
            if RULES.with_borrow(|r| r.len()) != before {
                VERSION.set(VERSION.get() + 1);
            }
            remove_unused();
        }
        notify();
    }

    /// Delete stored rules no longer in use (and the old single "wisp" list).
    fn remove_unused() {
        let Some(listing) = store() else { return };
        let handler = RcBlock::new(|ids: *mut NSArray<NSString>| {
            let Some(store) = store() else { return };
            // SAFETY: WebKit passes an array of strings, or null.
            let ids: Vec<String> = unsafe { ids.as_ref() }
                .map(|a| a.iter().map(|s| s.to_string()).collect())
                .unwrap_or_default();
            for id in ids {
                let ours = id == "wisp" || id.starts_with("wisp-");
                let used = RULES.with_borrow(|r| r.contains_key(&id))
                    || state(source_of(&id)).is_some_and(|s| s != State::Failed);
                if ours && !used {
                    // SAFETY: main thread, live store.
                    unsafe {
                        store.removeContentRuleListForIdentifier_completionHandler(
                            Some(&NSString::from_str(&id)),
                            Some(&RcBlock::new(|_: *mut NSError| {})),
                        )
                    };
                }
            }
        });
        // SAFETY: main thread, live store; the block lives until WebKit calls it.
        unsafe { listing.getAvailableContentRuleListIdentifiers(Some(&handler)) };
    }
}

// ---------------------------------------------------------------- Windows

#[cfg(windows)]
mod platform {
    use super::{ATTACHED, is_blocked, notify, sources, switch};
    use crate::{config::Adblock, filters, filters::matcher::Matcher};
    use std::sync::{Arc, RwLock};
    use webview2_com::{
        Microsoft::Web::WebView2::Win32::*, NavigationStartingEventHandler,
        WebResourceRequestedEventHandler, take_pwstr,
    };
    use windows_core::{HSTRING, Interface, PWSTR};
    use wry::{WebView, WebViewExtWindows};

    static MATCHER: RwLock<Option<Arc<Matcher>>> = RwLock::new(None);

    pub fn version() -> u64 {
        0 // the filter reads the live matcher; nothing to re-attach
    }

    pub fn key(webview: &WebView) -> usize {
        webview.webview().as_raw() as usize
    }

    pub fn lists_changed() {
        if let Some(cfg) = super::CFG.lock().unwrap().clone() {
            rebuild(&cfg);
        }
    }

    /// Build the matcher on a worker thread (about a tenth of a second for the default
    /// lists), then swap it in.
    pub fn rebuild(cfg: &Adblock) {
        let sources = sources(cfg);
        std::thread::spawn(move || {
            let mut m = Matcher::default();
            for rule in sources.iter().flat_map(|s| s.rules()) {
                if let filters::Rule::Net(n) = rule {
                    m.add(n);
                }
            }
            *MATCHER.write().unwrap() = Some(Arc::new(m));
            notify();
        });
    }

    fn kind(context: COREWEBVIEW2_WEB_RESOURCE_CONTEXT) -> u16 {
        match context {
            // Frames are not told apart from pages here, so path rules never stop a
            // frame on Windows; host rules still do (as on macOS).
            COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT => filters::DOC,
            COREWEBVIEW2_WEB_RESOURCE_CONTEXT_STYLESHEET => filters::STYLE,
            COREWEBVIEW2_WEB_RESOURCE_CONTEXT_IMAGE => filters::IMAGE,
            COREWEBVIEW2_WEB_RESOURCE_CONTEXT_MEDIA => filters::MEDIA,
            COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FONT => filters::FONT,
            COREWEBVIEW2_WEB_RESOURCE_CONTEXT_SCRIPT => filters::SCRIPT,
            COREWEBVIEW2_WEB_RESOURCE_CONTEXT_XML_HTTP_REQUEST
            | COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FETCH
            | COREWEBVIEW2_WEB_RESOURCE_CONTEXT_EVENT_SOURCE => filters::XHR,
            COREWEBVIEW2_WEB_RESOURCE_CONTEXT_WEBSOCKET => filters::SOCKET,
            COREWEBVIEW2_WEB_RESOURCE_CONTEXT_PING => filters::PING,
            _ => filters::OTHER,
        }
    }

    /// Answer every blocked request with an empty 403. Registered once per view; the
    /// view's on/off state is read per request.
    pub fn apply(webview: &WebView, key: usize, _on: bool, new: bool) {
        if !new {
            return;
        }
        let core = webview.webview();
        let env = webview.environment();
        let handler = WebResourceRequestedEventHandler::create(Box::new(move |sender, args| {
            let on = ATTACHED.with_borrow(|m| m.get(&key).is_some_and(|s| s.1));
            let (Some(args), true) = (args, on) else {
                return Ok(());
            };
            // SAFETY: plain COM calls on objects WebView2 passed to this callback.
            unsafe {
                let mut uri = PWSTR::null();
                args.Request()?.Uri(&mut uri)?;
                let uri = take_pwstr(uri);
                let mut page = PWSTR::null();
                let page = match sender {
                    Some(s) if s.Source(&mut page).is_ok() => Some(take_pwstr(page)),
                    _ => None,
                };
                let mut context = COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL;
                args.ResourceContext(&mut context)?;
                let matcher = MATCHER.read().unwrap().clone();
                let blocked = is_blocked(&uri)
                    || matcher.is_some_and(|m| m.blocks(&uri, page.as_deref(), kind(context)));
                if blocked {
                    let response = env.CreateWebResourceResponse(
                        None,
                        403,
                        &HSTRING::from("Blocked"),
                        &HSTRING::new(),
                    )?;
                    args.SetResponse(&response)?;
                }
            }
            Ok(())
        }));
        // The per-site switch, set before each page (main-frame) navigation goes out.
        let starting = NavigationStartingEventHandler::create(Box::new(move |_, args| {
            if let Some(args) = args {
                let mut uri = PWSTR::null();
                // SAFETY: COM call on the arguments WebView2 passed to this callback.
                unsafe { args.Uri(&mut uri)? };
                switch(key, &take_pwstr(uri), |_, _| {});
            }
            Ok(())
        }));
        // SAFETY: plain COM calls on a live webview.
        unsafe {
            let mut token = 0;
            let _ = core.add_NavigationStarting(&starting, &mut token);
            let _ = core.AddWebResourceRequestedFilter(
                &HSTRING::from("*"),
                COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
            );
            let _ = core.add_WebResourceRequested(&handler, &mut token);
        }
    }
}

// ---------------------------------------------------------------- other (Linux)

#[cfg(not(any(target_os = "macos", windows)))]
mod platform {
    // ponytail: no network-level blocking on Linux yet; only popups are filtered.
    // Upgrade path: WebKitGTK's UserContentFilterStore takes the same rule JSON as macOS.
    pub fn version() -> u64 {
        0
    }
    pub fn key(_webview: &wry::WebView) -> usize {
        0
    }
    pub fn apply(_webview: &wry::WebView, _key: usize, _on: bool, _new: bool) {}
    pub fn rebuild(_cfg: &crate::config::Adblock) {
        super::notify()
    }
    pub fn lists_changed() {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(enabled: bool, extra: &[&str], off: &[&str]) -> Adblock {
        Adblock {
            enabled,
            extra_hosts: extra.iter().map(|s| s.to_string()).collect(),
            lists: Vec::new(),
            off_sites: off.iter().map(|s| s.to_string()).collect(),
        }
    }

    // One test: this sets process-wide state.
    #[test]
    fn hosts_and_per_site_switch() {
        set_switches(&cfg(
            true,
            &["Ads.Example.org", "bad host!"],
            &["www.News.com"],
        ));
        assert!(is_blocked("https://doubleclick.net/x"));
        assert!(is_blocked("https://stats.g.doubleclick.net:443/x?y"));
        assert!(is_blocked("http://user@ads.example.org/"));
        assert!(is_blocked("https://u:p@doubleclick.net?x=1"));
        assert!(!is_blocked("https://example.org\\@doubleclick.net/"));
        assert!(!is_blocked("https://example.org/"));
        assert!(!is_blocked("https://notdoubleclick.net/"));
        assert!(!is_blocked(
            "https://example.com/?u=https://doubleclick.net/"
        ));
        assert!(!is_blocked("about:blank"));

        // The switch covers the site and its subdomains, and only those.
        assert!(!enabled_for("https://news.com/a"));
        assert!(!enabled_for("https://www.news.com/a"));
        assert!(!enabled_for("https://sport.NEWS.com/"));
        assert!(enabled_for("https://othernews.com/"));
        assert!(enabled_for("https://news.com.evil.org/"));
        assert!(enabled_for(""));

        set_switches(&cfg(false, &[], &[]));
        assert!(!is_blocked("https://doubleclick.net/x"));
        assert!(!enabled_for("https://example.org/"));
    }

    #[test]
    fn site_keys() {
        assert_eq!(site_key(" WWW.Example.com "), "example.com");
        assert_eq!(site_key("shop.example.com"), "shop.example.com");
    }

    #[test]
    fn refuses_bad_downloads() {
        let good: String = "[Adblock Plus 2.0]\n! Title: Test\n".to_string()
            + &(0..500)
                .map(|i| format!("||ad{i}.example.com^\n"))
                .collect::<String>();
        assert!(check(good.as_bytes(), None).is_ok());
        assert!(check(good.as_bytes(), Some(&good)).is_ok());
        // An error page, binary junk, a truncated copy, an oversized file.
        assert!(check(b"<!doctype html><title>502</title>", None).is_err());
        assert!(check(&[0xff, 0xfe, 0x00, 0x41], None).is_err());
        let half = &good[..good.len() / 3];
        assert!(check(half.as_bytes(), Some(&good)).is_err());
        assert!(check(b"[Adblock Plus 2.0]\n||a.com^\n", None).is_err());
        let huge = vec![b'!'; MAX_BYTES as usize + 1];
        assert!(check(&huge, None).is_err());
    }

    #[test]
    fn every_list_is_https() {
        for l in LISTS {
            assert!(l.url.starts_with("https://"), "{}", l.id);
        }
    }
}
