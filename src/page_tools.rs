//! Page tools: find in page, the downloads list, and the page's right-click menu.
//! The platform halves live in `native.rs`; this is the part that is the same everywhere.

use crate::{App, UserEvent, adblock, native};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    process::Command,
    rc::Rc,
};
use wry::WebViewBuilder;

// ---------------------------------------------------------------- downloads

#[derive(Clone, Copy, PartialEq, Debug)]
enum State {
    Active,
    Done,
    Failed,
}

struct Download {
    id: u32,
    /// The tab whose page started it.
    tab: u32,
    url: String,
    path: PathBuf,
    state: State,
}

/// What was downloaded since the app started. Nothing is kept between runs.
#[derive(Default)]
pub struct Downloads {
    items: Vec<Download>,
    next_id: u32,
}

/// A name for `name` in `dir` that `taken` does not hold yet: "a.pdf", "a (1).pdf", ...
/// Only the last part of `name` is used, so a name can never point outside `dir`.
fn unique_path(dir: &Path, name: &str, taken: impl Fn(&Path) -> bool) -> PathBuf {
    let name = Path::new(name)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("download");
    let (mut stem, mut ext) = match name.rfind('.') {
        Some(dot) if dot > 0 => name.split_at(dot),
        _ => (name, ""),
    };
    // "a.tar.gz" becomes "a (1).tar.gz", not "a.tar (1).gz"
    if let Some(inner) = stem.strip_suffix(".tar").filter(|s| !s.is_empty()) {
        (stem, ext) = (inner, &name[inner.len()..]);
    }
    let mut path = dir.join(name);
    let mut n = 1;
    while taken(&path) {
        path = dir.join(format!("{stem} ({n}){ext}"));
        n += 1;
    }
    path
}

impl Downloads {
    /// A download started by tab `tab` is in progress. That tab is not unloaded
    /// then: the engine may tie a download to the page that started it.
    pub fn running_in(&self, tab: u32) -> bool {
        self.items
            .iter()
            .any(|d| d.tab == tab && d.state == State::Active)
    }

    /// A download of `url` is starting. `suggested` is where the web engine would put
    /// it; only its last part, the file name, is believed (the server has a say in it).
    fn start(&mut self, tab: u32, url: &str, suggested: &Path) -> PathBuf {
        let home = std::env::home_dir().unwrap_or_default();
        // The folder is ours to choose. On Windows the Downloads folder can be moved,
        // and WebView2 itself (not the page) says where it is; elsewhere it is fixed.
        let dir = match suggested.parent() {
            Some(dir) if cfg!(windows) && dir.is_absolute() => dir.to_path_buf(),
            _ => home.join("Downloads"),
        };
        let name = suggested.file_name().and_then(|n| n.to_str());
        self.start_in(tab, &dir, url, name.unwrap_or("download"))
    }

    /// The download goes into `dir`, under a name no file or running download has.
    fn start_in(&mut self, tab: u32, dir: &Path, url: &str, name: &str) -> PathBuf {
        let _ = std::fs::create_dir_all(dir);
        let path = unique_path(dir, name, |p| {
            p.exists() || self.items.iter().any(|d| d.path == p)
        });
        self.next_id += 1;
        self.items.push(Download {
            id: self.next_id,
            tab,
            url: url.to_string(),
            path: path.clone(),
            state: State::Active,
        });
        path
    }

    /// The engine names a finished download by its address only, so the oldest one
    /// still running from that address is the one that ended.
    fn finish(&mut self, url: &str, ok: bool) {
        let running = |d: &&mut Download| d.state == State::Active && d.url == url;
        if let Some(d) = self.items.iter_mut().find(running) {
            d.state = if ok { State::Done } else { State::Failed };
        }
    }

    /// For the interface, newest first.
    pub fn json(&self) -> Value {
        let item = |d: &Download| {
            let name = d.path.file_name().unwrap_or_default().to_string_lossy();
            let state = match d.state {
                State::Active => "active",
                State::Done => "done",
                State::Failed => "failed",
            };
            json!({"id": d.id, "name": name, "state": state,
                   "host": adblock::host_of(&d.url).unwrap_or("")})
        };
        self.items.iter().rev().map(item).collect()
    }

    fn finished_path(&self, id: u32) -> Option<&Path> {
        let done = |d: &&Download| d.id == id && d.state == State::Done;
        self.items.iter().find(done).map(|d| d.path.as_path())
    }
}

/// Open a file with its usual program, or show it selected in its folder.
fn open_file(path: &Path, reveal: bool) {
    let mut cmd;
    if cfg!(target_os = "macos") {
        cmd = Command::new("/usr/bin/open");
        if reveal {
            cmd.arg("-R");
        }
        cmd.arg(path);
    } else if cfg!(windows) {
        cmd = Command::new("explorer.exe");
        if reveal {
            cmd.arg("/select,");
        }
        cmd.arg(path);
    } else {
        // ponytail: no "select this file" without a file-manager-specific call.
        cmd = Command::new("xdg-open");
        cmd.arg(if reveal {
            path.parent().unwrap_or(path)
        } else {
            path
        });
    }
    if let Err(e) = cmd.spawn() {
        eprintln!("wisp: could not open {}: {e}", path.display());
    }
}

// ---------------------------------------------------------------- the app's side

impl App {
    /// Give the webview of tab `tab` its downloads handling.
    pub(crate) fn with_downloads<'a>(
        &self,
        builder: WebViewBuilder<'a>,
        tab: u32,
    ) -> WebViewBuilder<'a> {
        let (list, list2) = (self.downloads.clone(), self.downloads.clone());
        let (p1, p2) = (self.proxy.clone(), self.proxy.clone());
        builder
            .with_download_started_handler(move |url, path| {
                *path = list.borrow_mut().start(tab, &url, path);
                let _ = p1.send_event(UserEvent::Downloads);
                true
            })
            .with_download_completed_handler(move |url, _, ok| {
                list2.borrow_mut().finish(&url, ok);
                let _ = p2.send_event(UserEvent::Downloads);
            })
    }

    /// A download started or ended. The first one puts the downloads button in the
    /// toolbar if the user has not placed it, so a download is never invisible.
    pub(crate) fn downloads_changed(&mut self) {
        let a = &mut self.cfg.layout.areas;
        let placed = [&a.hotbar, &a.before_tabs, &a.after_tabs]
            .iter()
            .any(|area| area.iter().any(|item| item == "downloads"));
        if !placed {
            let at = a.hotbar.iter().position(|item| item == "settings");
            a.hotbar
                .insert(at.unwrap_or(a.hotbar.len()), "downloads".into());
            self.cfg.save();
        }
        self.sync();
    }

    /// "Open" or "Show in folder" on a finished download, from the interface.
    pub(crate) fn download_op(&self, id: u32, op: &str) {
        if let Some(path) = self.downloads.borrow().finished_path(id) {
            open_file(path, op == "show");
        }
    }

    /// Search the page on screen for `query`: the first match, or the next/previous one.
    pub(crate) fn find(&mut self, query: String, back: bool) {
        let (Some(id), Some(webview)) = (self.active, self.active_webview()) else {
            return;
        };
        if query.is_empty() {
            native::find_stop(webview);
            self.find_last = None;
            return;
        }
        let key = (id, query);
        let fresh = self.find_last.as_ref() != Some(&key);
        let proxy = self.proxy.clone();
        native::find(webview, &key.1, back, fresh, move |found| {
            let _ = proxy.send_event(UserEvent::Found(id, found));
        });
        self.find_last = Some(key);
    }

    pub(crate) fn find_close(&mut self) {
        self.find_last = None;
        if let Some(webview) = self.active_webview() {
            native::find_stop(webview);
            if self.page_visible() {
                let _ = webview.focus();
            }
        }
    }

    pub(crate) fn found(&self, id: u32, found: native::Found) {
        if self.active == Some(id) {
            let result = json!({"found": found.found, "count": found.count});
            let _ = self
                .chrome
                .evaluate_script(&format!("wisp.found({result})"));
        }
    }
}

/// Shared between the app and the download handlers of every tab.
pub type SharedDownloads = Rc<RefCell<Downloads>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_never_collide_or_leave_the_folder() {
        let dir = Path::new("/d");
        let none = |_: &Path| false;
        assert_eq!(unique_path(dir, "a.pdf", none), dir.join("a.pdf"));
        let taken = |p: &Path| p == dir.join("a.pdf") || p == dir.join("a (1).pdf");
        assert_eq!(unique_path(dir, "a.pdf", taken), dir.join("a (2).pdf"));
        let taken = |p: &Path| p == dir.join("x.tar.gz") || p == dir.join("noext");
        assert_eq!(
            unique_path(dir, "x.tar.gz", taken),
            dir.join("x (1).tar.gz")
        );
        assert_eq!(unique_path(dir, "noext", taken), dir.join("noext (1)"));
        let taken = |p: &Path| p == dir.join(".env");
        assert_eq!(unique_path(dir, ".env", taken), dir.join(".env (1)"));
        // a hostile name stays inside the folder
        assert_eq!(
            unique_path(dir, "../../etc/passwd", none),
            dir.join("passwd")
        );
        assert_eq!(unique_path(dir, "..", none), dir.join("download"));
        assert_eq!(unique_path(dir, "", none), dir.join("download"));
    }

    #[test]
    fn downloads_do_not_overwrite_files_or_each_other() {
        let dir = std::env::temp_dir().join(format!("wisp-dl-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("report.pdf"), "already here").unwrap();

        let mut list = Downloads::default();
        let url = "https://files.example.com/report.pdf";
        // Two downloads of the same file while one copy is on disk: neither has
        // written anything yet, and still each gets its own name.
        let first = list.start_in(1, &dir, url, "report.pdf");
        let second = list.start_in(1, &dir, url, "../report.pdf");
        assert_eq!(first, dir.join("report (1).pdf"));
        assert_eq!(second, dir.join("report (2).pdf"));
        // Only the tab that started a download is kept loaded for it.
        assert!(list.running_in(1) && !list.running_in(2));
        assert_eq!(
            std::fs::read_to_string(dir.join("report.pdf")).unwrap(),
            "already here"
        );

        // They end in the order they started; a stray "finished" changes nothing.
        list.finish(url, true);
        list.finish(url, false);
        assert!(!list.running_in(1));
        list.finish(url, true);
        list.finish("https://other.example.com/x", true);
        let shown = list.json();
        assert_eq!(shown[0]["name"], "report (2).pdf"); // newest first
        assert_eq!(shown[0]["state"], "failed");
        assert_eq!(shown[1]["state"], "done");
        assert_eq!(shown[1]["host"], "files.example.com");
        // Only a finished file can be opened.
        assert_eq!(list.finished_path(1), Some(first.as_path()));
        assert_eq!(list.finished_path(2), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
