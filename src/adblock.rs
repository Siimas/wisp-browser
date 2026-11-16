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
    time::{Duration, SystemTime},
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

/// The rule sources for these settings: each downloaded list, the built-in hosts
/// while no list has been downloaded, and the user's extra hosts.
#[derive(Clone)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
struct Source {
    /// Names the compiled rules: changes whenever their content would.
    name: String,
    hosts: Vec<String>,
    list: Option<PathBuf>,
}

fn hash(x: impl Hash) -> u64 {
    let mut h = DefaultHasher::new();
    x.hash(&mut h);
    h.finish()
}

fn sources(cfg: &Adblock) -> Vec<Source> {
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
        let version = hash((meta.len(), stamp));
        out.push(Source {
            name: format!("wisp-{}-{version:x}-{FORMAT}", l.id),
            hosts: Vec::new(),
            list: Some(path),
        });
    }
    let hosts_source = |kind: &str, hosts: Vec<String>| Source {
        name: format!("wisp-{kind}-{:x}-{FORMAT}", hash(&hosts)),
        hosts,
        list: None,
    };
    if cfg.enabled && out.is_empty() {
        out.push(hosts_source("builtin", builtin_hosts()));
    }
    let extra = clean_hosts(cfg.extra_hosts.iter().map(String::as_str));
    if cfg.enabled && !extra.is_empty() {
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
    /// engine object's address; a new view always attaches first, so a reused address
    /// is overwritten before it is read.
    /// ponytail: entries of closed views stay (a few bytes each).
    static ATTACHED: RefCell<HashMap<usize, (u64, bool)>> = RefCell::new(HashMap::new());
}

/// Give this web view the current rules, on or off for the site at `url` (the UI
/// views pass ""). Call when a view is made, when its page changes, and on
/// `notify`. Returns true when blocking was switched for a page already loading,
/// which then needs a reload for the switch to take effect.
pub fn attach(webview: &wry::WebView, url: &str) -> bool {
    let on = enabled_for(url);
    let key = platform::key(webview);
    let now = (platform::version(), on);
    let before = ATTACHED.with_borrow_mut(|m| m.insert(key, now));
    if before != Some(now) {
        platform::apply(webview, key, on, before.is_none());
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
    if lists.is_empty() {
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
    // for a public file. Redirects to plain HTTP are refused.
    let client = reqwest::blocking::Client::builder()
        .https_only(true)
        .timeout(Duration::from_secs(60))
        .user_agent("Wisp")
        .build()
        .map_err(|e| e.to_string())?;
    let response = client.get(list.url).send().map_err(|e| e.to_string())?;
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
    use super::{Source, notify, sources};
    use crate::{config::Adblock, filters};
    use block2::RcBlock;
    use dispatch2::DispatchQueue;
    use objc2::{MainThreadMarker, rc::Retained};
    use objc2_foundation::{NSArray, NSError, NSString};
    use objc2_web_kit::{WKContentRuleList, WKContentRuleListStore};
    use std::{
        cell::{Cell, RefCell},
        collections::{BTreeMap, HashMap},
        time::Instant,
    };
    use wry::{WebView, WebViewExtMacOS};

    /// Rules per compiled list. WebKit takes up to 150,000 on current systems; smaller
    /// parts compile faster and stay under the 50,000 that older versions allowed.
    const PART: usize = 50_000;

    thread_local! {
        /// Compiled rule lists in use, by name (`<source>.<part>`).
        static RULES: RefCell<BTreeMap<String, Retained<WKContentRuleList>>> = RefCell::default();
        /// Sources wanted now, and how many of each one's parts are still on their way.
        static WANT: RefCell<HashMap<String, usize>> = RefCell::default();
        /// Lookups not answered yet; the first `notify` waits for them, not for compiles.
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

    pub fn version() -> u64 {
        VERSION.get()
    }

    pub fn key(webview: &WebView) -> usize {
        Retained::as_ptr(&webview.manager()) as usize
    }

    /// Put this view's rule lists in place: all of them, or none.
    pub fn apply(webview: &WebView, _key: usize, on: bool, _new: bool) {
        let manager = webview.manager();
        // SAFETY: main-thread calls on a live WKUserContentController.
        unsafe {
            manager.removeAllContentRuleLists();
            if on {
                RULES.with_borrow(|r| r.values().for_each(|l| manager.addContentRuleList(l)));
            }
        }
    }

    /// Called from the updater thread.
    pub fn lists_changed() {
        DispatchQueue::main().exec_async(|| {
            if let Some(cfg) = super::CFG.lock().unwrap().clone() {
                rebuild(&cfg);
            }
        });
    }

    /// Bring the compiled rules in line with the settings: rules already in use stay,
    /// stored ones are looked up, and only a list never seen in this version is
    /// converted (on a worker thread) and compiled. The old rules stay attached until
    /// every new source is ready, so a list update leaves no gap in blocking.
    pub fn rebuild(cfg: &Adblock) {
        let sources = sources(cfg);
        let loaded = |s: &Source| RULES.with_borrow(|r| r.keys().any(|k| source_of(k) == s.name));
        let (done, missing): (Vec<Source>, Vec<Source>) = sources.into_iter().partition(loaded);
        // Parts still to come per source; `usize::MAX` = not known yet.
        let want = done.iter().map(|s| (s.name.clone(), 0));
        WANT.set(
            want.chain(missing.iter().map(|s| (s.name.clone(), usize::MAX)))
                .collect(),
        );
        let Some(store) = store() else {
            return notify();
        };
        hold(1);
        let handler = RcBlock::new(move |ids: *mut NSArray<NSString>| {
            // SAFETY: WebKit passes an array of strings, or null.
            let ids: Vec<String> = unsafe { ids.as_ref() }
                .map(|a| a.iter().map(|s| s.to_string()).collect())
                .unwrap_or_default();
            remove_unused(&ids);
            for s in &missing {
                let parts: Vec<&String> = ids.iter().filter(|i| source_of(i) == s.name).collect();
                if parts.is_empty() {
                    build(s.clone());
                } else {
                    set_parts(&s.name, parts.len());
                    parts.into_iter().for_each(|id| look_up(id, s.clone()));
                }
            }
            release(1);
        });
        // SAFETY: main thread, live store; the block lives until WebKit calls it.
        unsafe { store.getAvailableContentRuleListIdentifiers(Some(&handler)) };
    }

    /// The first `notify` (when tabs start) waits for work held here: looking up
    /// stored rules, and compiling the small host lists. Big lists do not hold it up.
    fn hold(n: usize) {
        PENDING.set(PENDING.get() + n);
    }

    fn release(n: usize) {
        PENDING.set(PENDING.get().saturating_sub(n));
        settle();
    }

    fn set_parts(name: &str, n: usize) {
        WANT.with_borrow_mut(|w| {
            if let Some(left) = w.get_mut(name) {
                *left = n;
            }
        });
    }

    fn look_up(id: &str, source: Source) {
        let Some(store) = store() else { return };
        let id_owned = id.to_string();
        hold(1);
        let handler = RcBlock::new(move |list: *mut WKContentRuleList, _: *mut NSError| {
            // SAFETY: WebKit hands us a valid rule list or null.
            match unsafe { Retained::retain(list) } {
                Some(list) => loaded(&id_owned, list),
                // Stored by another WebKit version, say: build it again (once).
                None => {
                    let building = WANT.with_borrow(|w| w.get(&source.name) == Some(&usize::MAX));
                    if !building {
                        set_parts(&source.name, usize::MAX);
                        build(source.clone());
                    }
                }
            }
            release(1);
        });
        // SAFETY: as above.
        unsafe {
            store.lookUpContentRuleListForIdentifier_completionHandler(
                Some(&NSString::from_str(id)),
                Some(&handler),
            )
        };
    }

    /// Convert a source on a worker thread, then compile its parts here.
    fn build(s: Source) {
        let small = s.list.is_none();
        if small {
            hold(1);
        }
        std::thread::spawn(move || {
            let started = Instant::now();
            let parts = filters::to_webkit(s.rules().into_iter(), PART);
            let took = started.elapsed();
            DispatchQueue::main().exec_async(move || {
                eprintln!(
                    "wisp: {} converted in {took:?} ({} parts)",
                    s.name,
                    parts.len()
                );
                let wanted = WANT.with_borrow(|w| w.contains_key(&s.name));
                if wanted {
                    set_parts(&s.name, parts.len());
                    if small {
                        hold(parts.len());
                    }
                    for (i, json) in parts.into_iter().enumerate() {
                        compile(format!("{}.{i}", s.name), json, small);
                    }
                }
                if small { release(1) } else { settle() }
            });
        });
    }

    fn compile(id: String, json: String, small: bool) {
        let Some(store) = store() else { return };
        let started = Instant::now();
        let size = json.len();
        let name = id.clone();
        let handler = RcBlock::new(move |list: *mut WKContentRuleList, err: *mut NSError| {
            // SAFETY: as above.
            match unsafe { Retained::retain(list) } {
                Some(list) => {
                    eprintln!(
                        "wisp: compiled {name} ({} KB) in {:?}",
                        size / 1024,
                        started.elapsed()
                    );
                    loaded(&name, list);
                }
                None => {
                    // SAFETY: WebKit passes an error with a null list.
                    let why = unsafe { err.as_ref() }.map(|e| e.localizedDescription().to_string());
                    eprintln!("wisp: ad block rules {name} failed to compile: {why:?}");
                    part_done(&name);
                }
            }
            if small { release(1) } else { settle() }
        });
        // SAFETY: as above.
        unsafe {
            store.compileContentRuleListForIdentifier_encodedContentRuleList_completionHandler(
                Some(&NSString::from_str(&id)),
                Some(&NSString::from_str(&json)),
                Some(&handler),
            )
        };
    }

    fn loaded(id: &str, list: Retained<WKContentRuleList>) {
        if WANT.with_borrow(|w| w.contains_key(source_of(id))) {
            RULES.with_borrow_mut(|r| r.insert(id.to_string(), list));
            VERSION.set(VERSION.get() + 1);
        }
        part_done(id);
    }

    fn part_done(id: &str) {
        WANT.with_borrow_mut(|w| {
            if let Some(n) = w.get_mut(source_of(id)).filter(|n| **n != usize::MAX) {
                *n = n.saturating_sub(1);
            }
        });
    }

    /// Once every wanted source is complete, drop rules no longer wanted; tell the app
    /// whenever nothing is waiting on a lookup (the first time, tabs start loading).
    fn settle() {
        if PENDING.get() > 0 {
            return;
        }
        if WANT.with_borrow(|w| w.values().all(|n| *n == 0)) {
            let before = RULES.with_borrow(|r| r.len());
            RULES.with_borrow_mut(|r| {
                r.retain(|k, _| WANT.with_borrow(|w| w.contains_key(source_of(k))))
            });
            if RULES.with_borrow(|r| r.len()) != before {
                VERSION.set(VERSION.get() + 1);
            }
        }
        notify();
    }

    /// Delete stored rules of list versions no longer in use (and the old single
    /// "wisp" list), so the store does not grow with every update.
    fn remove_unused(ids: &[String]) {
        let Some(store) = store() else { return };
        for id in ids {
            let ours = id == "wisp" || id.starts_with("wisp-");
            if ours && !WANT.with_borrow(|w| w.contains_key(source_of(id))) {
                // SAFETY: as above; no completion handler needed.
                unsafe {
                    store.removeContentRuleListForIdentifier_completionHandler(
                        Some(&NSString::from_str(id)),
                        Some(&RcBlock::new(|_: *mut NSError| {})),
                    )
                };
            }
        }
    }
}

// ---------------------------------------------------------------- Windows

#[cfg(windows)]
mod platform {
    use super::{ATTACHED, is_blocked, notify, sources};
    use crate::{config::Adblock, filters, filters::matcher::Matcher};
    use std::sync::{Arc, RwLock};
    use webview2_com::{
        Microsoft::Web::WebView2::Win32::*, WebResourceRequestedEventHandler, take_pwstr,
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
        // SAFETY: plain COM calls on a live webview.
        unsafe {
            let mut token = 0;
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
