//! History and bookmarks: two plain JSON files beside `config.json`, the matching
//! behind the address bar's suggestions, and (at the bottom) the glue that connects
//! them to the window.
//!
//! History keeps one row per address: its title, how many times it was visited and
//! when it was last visited. Private tabs are never recorded, and a private window
//! opens the store read-only: it can suggest from what is there but changes nothing.
//!
//! ponytail: each file is rewritten whole on every change (about 1 MB at the history
//! cap), and a second normal window (a second copy of the app) would overwrite the
//! first one's file. Upgrade path: an append-only log, or SQLite.

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::HashSet,
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

/// History size cap: this many distinct addresses. One more and the address visited
/// longest ago is dropped.
pub const MAX_VISITS: usize = 5000;
/// Rows offered under the address bar, not counting the final "Search for" row.
const MAX_SUGGESTIONS: usize = 7;
/// Of those, at most this many are open tabs.
const MAX_TAB_SUGGESTIONS: usize = 3;
/// Rows sent to the History sheet at once; searching reaches the older ones.
const SHEET_ROWS: usize = 500;
/// Longer addresses (pages that carry their content in the address) are not kept.
const MAX_URL: usize = 2000;
const MAX_TITLE: usize = 300;
const DAY: f64 = 86_400.0;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Visit {
    pub url: String,
    #[serde(default)]
    pub title: String,
    pub count: u32,
    /// Last visit, seconds since 1970.
    pub last: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Bookmark {
    pub url: String,
    #[serde(default)]
    pub title: String,
    /// When it was added, seconds since 1970.
    pub added: u64,
}

/// One row under the address bar.
#[derive(Serialize, Debug, PartialEq)]
pub struct Suggestion {
    /// "tab" (switch to it), "bookmark", "history" or "search".
    pub kind: &'static str,
    pub url: String,
    pub title: String,
    /// For "tab": which one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tab: Option<u32>,
}

#[derive(Default)]
pub struct Store {
    /// Where the files live; `None` keeps everything in memory (tests).
    dir: Option<PathBuf>,
    /// A private window: nothing is changed and nothing is written.
    read_only: bool,
    visits: Vec<Visit>,
    bookmarks: Vec<Bookmark>,
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn web(url: &str) -> bool {
    (url.starts_with("http://") || url.starts_with("https://")) && url.len() <= MAX_URL
}

fn clip(title: &str) -> String {
    title.chars().take(MAX_TITLE).collect()
}

/// The address as people type it: no scheme, no "www.".
fn bare(lower_url: &str) -> &str {
    let s = lower_url
        .strip_prefix("https://")
        .or_else(|| lower_url.strip_prefix("http://"))
        .unwrap_or(lower_url);
    s.strip_prefix("www.").unwrap_or(s)
}

/// How well a row answers what was typed, or `None` if it does not: every word must
/// appear in the address or the title; typing the start of the site's name counts triple.
fn fit(query: &str, url: &str, title: &str) -> Option<f64> {
    let (q, url, title) = (
        query.trim().to_lowercase(),
        url.to_lowercase(),
        title.to_lowercase(),
    );
    let found = q
        .split_whitespace()
        .all(|w| url.contains(w) || title.contains(w));
    (found && !q.is_empty()).then(|| if bare(&url).starts_with(&q) { 3.0 } else { 1.0 })
}

/// "Frecency": grows slowly with the number of visits and fades with time, halving
/// after a week away and continuing to fall. Twenty visits a month ago rank below
/// three visits today.
fn frecency(count: u32, last: u64, now: u64) -> f64 {
    let days = now.saturating_sub(last) as f64 / DAY;
    (1.0 + count as f64).ln() / (1.0 + days / 7.0)
}

impl Store {
    pub fn load(dir: PathBuf, read_only: bool) -> Self {
        fn read<T: for<'a> Deserialize<'a>>(dir: &std::path::Path, name: &str) -> Vec<T> {
            let path = dir.join(name);
            let Ok(text) = fs::read_to_string(&path) else {
                return Vec::new();
            };
            serde_json::from_str(&text).unwrap_or_else(|e| {
                // Unreadable: keep it aside rather than overwrite it with an empty list.
                eprintln!("wisp: could not read {name}: {e}");
                let _ = fs::copy(&path, dir.join(format!("{name}.bad")));
                Vec::new()
            })
        }
        Self {
            visits: read(&dir, "history.json"),
            bookmarks: read(&dir, "bookmarks.json"),
            dir: Some(dir),
            read_only,
        }
    }

    fn save<T: Serialize>(&self, name: &str, rows: &[T]) {
        let Some(dir) = self.dir.as_ref().filter(|_| !self.read_only) else {
            return;
        };
        // Written beside the real file and moved over it, so a crash mid-write
        // cannot leave half a file.
        let (tmp, path) = (dir.join(format!("{name}.tmp")), dir.join(name));
        let _ = fs::create_dir_all(dir);
        let written = fs::write(&tmp, serde_json::to_string(rows).unwrap_or_default())
            .and_then(|()| fs::rename(&tmp, &path));
        if let Err(e) = written {
            eprintln!("wisp: could not write {name}: {e}");
        }
    }

    fn save_history(&self) {
        self.save("history.json", &self.visits);
    }

    fn save_bookmarks(&self) {
        self.save("bookmarks.json", &self.bookmarks);
    }

    /// A page finished loading in a tab. `private` = that tab is private: not recorded.
    pub fn record(&mut self, url: &str, title: &str, private: bool, now: u64) {
        if private || self.read_only || !web(url) {
            return;
        }
        match self.visits.iter_mut().find(|v| v.url == url) {
            Some(v) => {
                v.count = v.count.saturating_add(1);
                v.last = now;
                if !title.is_empty() {
                    v.title = clip(title);
                }
            }
            None => self.visits.push(Visit {
                url: url.into(),
                title: clip(title),
                count: 1,
                last: now,
            }),
        }
        while self.visits.len() > MAX_VISITS {
            let oldest = (0..self.visits.len()).min_by_key(|&i| self.visits[i].last);
            self.visits.swap_remove(oldest.unwrap_or(0));
        }
        self.save_history();
    }

    /// A page already in history announced its title (often after it finished loading).
    pub fn retitle(&mut self, url: &str, title: &str, private: bool) {
        if private || self.read_only || title.is_empty() {
            return;
        }
        let title = clip(title);
        if let Some(v) = self.visits.iter_mut().find(|v| v.url == url)
            && v.title != title
        {
            v.title = title;
            self.save_history();
        }
    }

    pub fn forget(&mut self, url: &str) {
        if !self.read_only {
            self.visits.retain(|v| v.url != url);
            self.save_history();
        }
    }

    /// Drop every page last visited at or after `since` (0 = everything).
    pub fn clear_since(&mut self, since: u64) {
        if !self.read_only {
            self.visits.retain(|v| v.last < since);
            self.save_history();
        }
    }

    /// History for the sheet: latest first, filtered by `query` when there is one.
    pub fn history(&self, query: &str) -> Vec<&Visit> {
        let all = query.trim().is_empty();
        let mut rows: Vec<&Visit> = self
            .visits
            .iter()
            .filter(|v| all || fit(query, &v.url, &v.title).is_some())
            .collect();
        rows.sort_by_key(|v| std::cmp::Reverse(v.last));
        rows.truncate(SHEET_ROWS);
        rows
    }

    pub fn is_bookmarked(&self, url: &str) -> bool {
        self.bookmarks.iter().any(|b| b.url == url)
    }

    /// Add the page, or remove it if it is already there.
    pub fn toggle_bookmark(&mut self, url: &str, title: &str, now: u64) {
        if self.read_only {
            return;
        }
        if self.is_bookmarked(url) {
            self.bookmarks.retain(|b| b.url != url);
        } else if web(url) {
            self.bookmarks.push(Bookmark {
                url: url.into(),
                title: clip(title),
                added: now,
            });
        }
        self.save_bookmarks();
    }

    pub fn rename_bookmark(&mut self, url: &str, title: &str) {
        if self.read_only {
            return;
        }
        if let Some(b) = self.bookmarks.iter_mut().find(|b| b.url == url) {
            b.title = clip(title.trim());
            self.save_bookmarks();
        }
    }

    pub fn remove_bookmark(&mut self, url: &str) {
        if !self.read_only && self.is_bookmarked(url) {
            self.toggle_bookmark(url, "", 0);
        }
    }

    /// Bookmarks for the sheet: newest first, filtered by `query` when there is one.
    pub fn bookmarks(&self, query: &str) -> Vec<&Bookmark> {
        let all = query.trim().is_empty();
        let mut rows: Vec<&Bookmark> = self
            .bookmarks
            .iter()
            .filter(|b| all || fit(query, &b.url, &b.title).is_some())
            .collect();
        rows.sort_by_key(|b| std::cmp::Reverse(b.added));
        rows
    }

    /// What to offer for `query`: matching open tabs first (`tabs` = id, address,
    /// title of every tab but the one on screen), then bookmarks and history together,
    /// best first. A bookmark counts double; each address appears once.
    pub fn suggest(&self, query: &str, tabs: &[(u32, &str, &str)], now: u64) -> Vec<Suggestion> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for &(id, url, title) in tabs {
            if out.len() < MAX_TAB_SUGGESTIONS
                && web(url)
                && fit(query, url, title).is_some()
                && seen.insert(url)
            {
                out.push(Suggestion {
                    kind: "tab",
                    url: url.into(),
                    title: title.into(),
                    tab: Some(id),
                });
            }
        }
        // (score, kind, address, title)
        let mut ranked: Vec<(f64, &'static str, &str, &str)> = Vec::new();
        for b in &self.bookmarks {
            if let Some(fit) = fit(query, &b.url, &b.title) {
                let visit = self.visits.iter().find(|v| v.url == b.url);
                let (count, last) = visit.map_or((1, b.added), |v| (v.count, v.last));
                let score = 2.0 * fit * frecency(count, last, now);
                ranked.push((score, "bookmark", &b.url, &b.title));
            }
        }
        for v in &self.visits {
            if let Some(fit) = fit(query, &v.url, &v.title)
                && !self.is_bookmarked(&v.url)
            {
                let score = fit * frecency(v.count, v.last, now);
                ranked.push((score, "history", &v.url, &v.title));
            }
        }
        ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
        for (_, kind, url, title) in ranked {
            if out.len() < MAX_SUGGESTIONS && seen.insert(url) {
                out.push(Suggestion {
                    kind,
                    url: url.into(),
                    title: title.into(),
                    tab: None,
                });
            }
        }
        out
    }
}

// ================= WINDOW GLUE =================
// Everything the window does with the store. `main.rs` only forwards to these.

/// Messages from the interface about history, bookmarks and suggestions.
#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Msg {
    /// The address bar wants suggestions; answered with `wisp.suggestions(query, rows)`.
    Suggest {
        query: String,
    },
    /// The suggestions list of a floating bar opened or closed: it needs room below the bar.
    Grow {
        on: bool,
    },
    /// The History sheet wants its rows; answered with `wisp.library("history", rows)`.
    History {
        query: String,
    },
    /// Likewise for the Bookmarks sheet.
    Bookmarks {
        query: String,
    },
    /// Delete one history entry.
    Forget {
        url: String,
    },
    /// Clear history of the last `secs` seconds (0 = all of it) and, with `site_data`,
    /// cookies and everything else sites have stored.
    ClearData {
        secs: u64,
        site_data: bool,
    },
    RenameBookmark {
        url: String,
        title: String,
    },
    RemoveBookmark {
        url: String,
    },
}

impl crate::App {
    /// The actions `history`, `bookmarks` and `bookmark_page`.
    pub(crate) fn library_action(&mut self, name: &str) {
        if name == "bookmark_page" {
            if let Some(tab) = self
                .active
                .and_then(|id| self.tabs.iter().find(|t| t.id == id))
            {
                self.store.toggle_bookmark(&tab.url, &tab.title, now());
            }
            return self.sync();
        }
        // The sheets belong to the interface; it shows and hides them (and the page).
        if self.settings_open {
            self.set_settings_open(false);
        }
        let _ = self.chrome.focus();
        let script = format!("wisp.sheet({})", json!(name));
        let _ = self.chrome.evaluate_script(&script);
    }

    /// A page finished loading in tab `id`.
    pub(crate) fn visited(&mut self, id: u32) {
        if let Some(t) = self.tabs.iter().find(|t| t.id == id) {
            self.store.record(&t.url, &t.title, t.private, now());
        }
    }

    /// The page in tab `id` changed its title.
    pub(crate) fn retitled(&mut self, id: u32) {
        if let Some(t) = self.tabs.iter().find(|t| t.id == id) {
            self.store.retitle(&t.url, &t.title, t.private);
        }
    }

    pub(crate) fn library(&mut self, msg: Msg) {
        match msg {
            Msg::Suggest { query } => {
                let tabs: Vec<(u32, &str, &str)> = self
                    .tabs
                    .iter()
                    .filter(|t| Some(t.id) != self.active)
                    .map(|t| (t.id, t.url.as_str(), t.title.as_str()))
                    .collect();
                let mut rows = self.store.suggest(&query, &tabs, now());
                rows.push(Suggestion {
                    kind: "search",
                    url: crate::config::search(query.trim(), &self.cfg.behavior.search_url),
                    title: query.trim().to_string(),
                    tab: None,
                });
                // Both copies of the interface hear it; the one that asked shows it.
                let js = format!("wisp.suggestions({}, {})", json!(query), json!(rows));
                for ui in self.uis() {
                    let _ = ui.evaluate_script(&js);
                }
            }
            Msg::Grow { on } => {
                if let Some(f) = self.floater.as_ref().filter(|_| self.peek) {
                    let [l, t, r, b] = self.float_insets;
                    let _ = f.set_bounds(self.inset_rect([l, t, r, if on { 0.0 } else { b }]));
                }
            }
            Msg::History { query } => {
                let rows = json!(self.store.history(&query));
                let _ = self
                    .chrome
                    .evaluate_script(&format!("wisp.library('history', {rows})"));
            }
            Msg::Bookmarks { query } => {
                let rows = json!(self.store.bookmarks(&query));
                let _ = self
                    .chrome
                    .evaluate_script(&format!("wisp.library('bookmarks', {rows})"));
            }
            Msg::Forget { url } => self.store.forget(&url),
            Msg::ClearData { secs, site_data } => {
                if self.private_window {
                    return;
                }
                let since = if secs == 0 {
                    0
                } else {
                    now().saturating_sub(secs)
                };
                self.store.clear_since(since);
                if site_data {
                    // The engine clears all of it or none: there is no time range here.
                    let tabs = self.tabs.iter().filter(|t| !t.private);
                    let pages = tabs.filter_map(|t| t.webview.as_ref());
                    for w in std::iter::once(&self.chrome).chain(pages) {
                        if let Err(e) = w.clear_all_browsing_data() {
                            eprintln!("wisp: could not clear site data: {e}");
                        }
                    }
                }
            }
            Msg::RenameBookmark { url, title } => self.store.rename_bookmark(&url, &title),
            Msg::RemoveBookmark { url } => {
                self.store.remove_bookmark(&url);
                self.sync(); // the bookmark button may have to empty
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000;
    const DAY: u64 = 86_400;

    fn store(rows: &[(&str, &str, u32, u64)]) -> Store {
        let mut s = Store::default();
        for &(url, title, count, days_ago) in rows {
            for _ in 0..count {
                s.record(url, title, false, NOW - days_ago * DAY);
            }
        }
        s
    }

    fn urls(rows: &[Suggestion]) -> Vec<&str> {
        rows.iter().map(|r| r.url.as_str()).collect()
    }

    #[test]
    fn ranking_weighs_how_often_against_how_recently() {
        let s = store(&[
            ("https://news.test/old", "News archive", 20, 30),
            ("https://news.test/today", "News today", 3, 0),
            ("https://news.test/once", "News once", 1, 2),
            ("https://other.test/", "Unrelated", 50, 0),
        ]);
        let got = s.suggest("news", &[], NOW);
        assert_eq!(
            urls(&got),
            [
                "https://news.test/today",
                "https://news.test/old",
                "https://news.test/once"
            ]
        );
        // Same recency: the page visited more often wins.
        let s = store(&[
            ("https://a.test/rare", "Docs", 1, 1),
            ("https://a.test/often", "Docs", 9, 1),
        ]);
        assert_eq!(
            urls(&s.suggest("docs", &[], NOW))[0],
            "https://a.test/often"
        );
    }

    #[test]
    fn matching_and_the_order_of_kinds() {
        let mut s = store(&[
            ("https://www.rust-lang.org/learn", "Learn Rust", 2, 0),
            ("https://blog.test/rust-tips", "Tips for Rust", 9, 0),
            (
                "https://github.com/rust-lang/rust",
                "The Rust repository",
                1,
                0,
            ),
        ]);
        // Typing the start of a site's name beats a busier page that merely mentions it.
        assert_eq!(
            urls(&s.suggest("rust", &[], NOW))[0],
            "https://www.rust-lang.org/learn"
        );
        // Every word must be there; case does not matter; titles count.
        assert_eq!(
            urls(&s.suggest("TIPS rust", &[], NOW)),
            ["https://blog.test/rust-tips"]
        );
        assert!(s.suggest("rust python", &[], NOW).is_empty());
        assert!(s.suggest("  ", &[], NOW).is_empty());

        // A bookmark counts double, and an open tab comes first and is not repeated.
        s.toggle_bookmark("https://github.com/rust-lang/rust", "Rust repo", NOW);
        let tabs = [
            (7, "https://blog.test/rust-tips", "Tips for Rust"),
            (8, "about:blank", "rust"),
        ];
        let got = s.suggest("rust", &tabs, NOW);
        let kinds: Vec<_> = got.iter().map(|r| (r.kind, r.url.as_str())).collect();
        assert_eq!(
            kinds,
            [
                ("tab", "https://blog.test/rust-tips"),
                ("history", "https://www.rust-lang.org/learn"),
                ("bookmark", "https://github.com/rust-lang/rust"),
            ]
        );
        assert_eq!(got[0].tab, Some(7));
        // Never more rows than the list has room for.
        let many: Vec<_> = (0..40)
            .map(|i| (format!("https://m.test/{i}"), "Many", 1, 0))
            .collect();
        let rows: Vec<_> = many
            .iter()
            .map(|(u, t, c, d)| (u.as_str(), *t, *c, *d))
            .collect();
        assert_eq!(
            store(&rows).suggest("many", &[], NOW).len(),
            MAX_SUGGESTIONS
        );
    }

    #[test]
    fn history_is_capped_by_dropping_the_page_visited_longest_ago() {
        let mut s = Store::default();
        for i in 0..MAX_VISITS as u64 {
            s.record(&format!("https://cap.test/{i}"), "", false, 1000 + i);
        }
        s.record("https://cap.test/0", "", false, 900_000); // seen again: now the newest
        s.record("https://cap.test/new", "", false, 900_001);
        assert_eq!(s.visits.len(), MAX_VISITS);
        let has = |u: &str| s.visits.iter().any(|v| v.url == u);
        assert!(has("https://cap.test/0") && has("https://cap.test/new"));
        assert!(!has("https://cap.test/1")); // the oldest went
        assert!(has("https://cap.test/2"));
    }

    #[test]
    fn private_tabs_and_private_windows_leave_no_trace() {
        let dir = std::env::temp_dir().join(format!("wisp-store-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);

        // A normal window: the private tab is skipped, the normal one is written.
        let mut s = Store::load(dir.clone(), false);
        s.record("https://secret.test/", "Secret", true, NOW);
        s.retitle("https://secret.test/", "Secret", true);
        assert!(s.visits.is_empty() && !dir.join("history.json").exists());
        s.record("https://open.test/", "Open", false, NOW);
        s.record("https://open.test/", "", false, NOW + 5); // a reload keeps the title
        s.record("file:///etc/hosts", "Not the web", false, NOW);
        s.toggle_bookmark("https://open.test/", "Open", NOW);
        let saved = fs::read_to_string(dir.join("history.json")).unwrap();
        assert!(saved.contains("open.test") && !saved.contains("secret"));
        assert_eq!((s.visits.len(), s.visits[0].count), (1, 2));
        assert_eq!(s.visits[0].title, "Open");

        // A private window sees what is there, changes nothing and writes nothing.
        let before = (
            saved,
            fs::read_to_string(dir.join("bookmarks.json")).unwrap(),
        );
        let mut p = Store::load(dir.clone(), true);
        assert_eq!(p.suggest("open", &[], NOW).len(), 1);
        p.record("https://private-window.test/", "", false, NOW);
        p.toggle_bookmark("https://private-window.test/", "", NOW);
        p.toggle_bookmark("https://open.test/", "", NOW);
        p.rename_bookmark("https://open.test/", "Renamed");
        p.remove_bookmark("https://open.test/");
        p.forget("https://open.test/");
        p.clear_since(0);
        assert_eq!((p.visits.len(), p.bookmarks.len()), (1, 1));
        let after = (
            fs::read_to_string(dir.join("history.json")).unwrap(),
            fs::read_to_string(dir.join("bookmarks.json")).unwrap(),
        );
        assert_eq!(before, after);

        // What was written comes back.
        let again = Store::load(dir.clone(), false);
        assert_eq!((again.visits, again.bookmarks), (s.visits, s.bookmarks));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn bookmarks_and_clearing() {
        let mut s = store(&[
            ("https://a.test/", "A", 1, 0),
            ("https://b.test/", "B", 1, 3),
            ("https://c.test/", "C", 1, 40),
        ]);
        let listed =
            |s: &Store| -> Vec<String> { s.history("").iter().map(|v| v.title.clone()).collect() };
        assert_eq!(listed(&s), ["A", "B", "C"]); // latest first
        assert_eq!(s.history("b.test").len(), 1);
        s.clear_since(NOW - 7 * DAY); // "the last 7 days"
        assert_eq!(listed(&s), ["C"]);
        s.forget("https://c.test/");
        assert!(s.history("").is_empty());

        s.toggle_bookmark("https://a.test/", "A", NOW);
        s.toggle_bookmark("javascript:alert(1)", "No", NOW);
        assert!(s.is_bookmarked("https://a.test/") && s.bookmarks.len() == 1);
        s.rename_bookmark("https://a.test/", "  Alpha ");
        assert_eq!(s.bookmarks("alpha")[0].title, "Alpha");
        s.toggle_bookmark("https://a.test/", "A", NOW);
        assert!(s.bookmarks("").is_empty());
    }
}
