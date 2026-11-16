#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! Wisp: a small browser shell around the operating system's own web engine.
//!
//! The window holds three kinds of webview, bottom to top:
//! 1. the interface ("chrome"), filling the whole window: hotbar, sidebar, settings;
//! 2. one webview per tab. The interface reports the gap it leaves for the page
//!    (`Msg::PageRect`) and the active tab is placed exactly there, as a rounded card;
//! 3. the "floater": a second copy of the interface, shown only while a collapsed
//!    bar is revealed, so that the bar can float over the page.

mod adblock;
mod config;
#[cfg(all(target_os = "macos", debug_assertions))]
mod debug_shot;
mod native;
mod update;

use config::{Config, SavedTab, Session};
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use tao::{
    dpi::{LogicalPosition, LogicalSize},
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    window::{Theme, Window, WindowBuilder},
};
use wry::{NewWindowResponse, PageLoadEvent, Rect, WebView, WebViewBuilder};

// Collapsed bar: how the cursor brings it back and lets it go again.
/// Overshooting the window edge by up to this much still counts as reaching it, so
/// the bar does not slip away when the window is not at the edge of the screen.
const OVERSHOOT: f64 = 120.0;
/// Cursor may stray this far past the bar before it counts as having left.
const LEAVE_MARGIN: f64 = 36.0;
/// How long the cursor must stay away before the bar hides again.
const LEAVE_DELAY: Duration = Duration::from_millis(180);
/// How often the cursor is checked while the bar is collapsed.
const POLL: Duration = Duration::from_millis(25);
/// Look for a new version this often (and once shortly after start).
const UPDATE_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// The page card gliding to a new size (keep in step with --d-3 in ui.html).
const PAGE_GLIDE: Duration = Duration::from_millis(220);
const FRAME: Duration = Duration::from_millis(8);

/// Messages from the UI (and, for `Key` only, from pages).
#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum Msg {
    Ready,
    Drag,
    Maximize,
    Key {
        combo: String,
    },
    Action {
        name: String,
    },
    Navigate {
        url: String,
    },
    SelectTab {
        id: u32,
    },
    CloseTab {
        id: u32,
    },
    MoveTab {
        id: u32,
        workspace: u32,
    },
    SelectWorkspace {
        id: u32,
    },
    /// Drag to reorder: put a tab at `index` among its workspace's tabs.
    ReorderTab {
        id: u32,
        index: usize,
    },
    PinTab {
        id: u32,
        pinned: bool,
    },
    DuplicateTab {
        id: u32,
    },
    CloseOtherTabs {
        id: u32,
    },
    /// Where things go, as distances from each window edge (left, top, right, bottom):
    /// the gap left for the page, and the area a floating bar would take.
    PageRect {
        page: [f64; 4],
        float: [f64; 4],
        /// Corner radius of the page card.
        radius: f64,
        /// The floating bar's visible panel inside `float` (same form), and its corner radius.
        #[serde(default)]
        panel: [f64; 4],
        #[serde(default)]
        panel_radius: f64,
        /// macOS: where the window buttons go: x of the first one, and the vertical
        /// center of the row they share with the bar.
        #[serde(default)]
        lights: [f64; 2],
        /// Glide to the new size (a layout change) instead of jumping.
        animate: bool,
    },
    /// The interface needs the page area for itself (a menu or sheet is open).
    Cover {
        on: bool,
    },
    /// The user is typing in the bar; do not hide a revealed bar under them.
    Busy {
        on: bool,
    },
    /// Reveal the collapsed bar.
    Peek {
        on: bool,
    },
    /// The address bar is not on screen: reveal the bar and focus it there.
    PeekFocus,
    /// The floating bar finished sliding away.
    FloaterDone,
    SaveSettings {
        config: Box<Config>,
    },
}

enum UserEvent {
    Ui(Msg),
    /// Hotkey reported by the page in tab `.0`.
    PageKey(u32, String),
    Title(u32, String),
    Load(u32, bool, String),
    /// A page in tab `.0` asked for a new window.
    NewTab(u32, String),
    AdblockReady,
    /// An update check finished: a version ready to install, none, or why it failed.
    Update(Result<Option<Box<update::Ready>>, String>),
}

struct Tab {
    id: u32,
    workspace: u32,
    private: bool,
    /// Pinned tabs are listed before the others of their workspace.
    pinned: bool,
    /// `None` until the tab is first shown: restored tabs cost nothing until used.
    webview: Option<WebView>,
    title: String,
    url: String,
    loading: bool,
    zoom: f64,
}

/// A closed tab, remembered for this run so it can be reopened.
struct Closed {
    url: String,
    title: String,
    workspace: u32,
    pinned: bool,
    /// Its place among its workspace's tabs.
    index: usize,
}

/// How many closed tabs are remembered.
const CLOSED_MAX: usize = 25;

/// Put tab `id` at `index` among the tabs of its workspace (the order the interface
/// lists them in), without leaving its group: pinned tabs first, then the rest.
fn place_tab(tabs: &mut Vec<Tab>, id: u32, index: usize) {
    let Some(pos) = tabs.iter().position(|t| t.id == id) else {
        return;
    };
    let tab = tabs.remove(pos);
    // Where the workspace's other tabs sit in the whole list.
    let peers: Vec<usize> = (0..tabs.len())
        .filter(|&i| tabs[i].workspace == tab.workspace)
        .collect();
    let pinned = peers.iter().filter(|&&i| tabs[i].pinned).count();
    let index = if tab.pinned {
        index.min(pinned)
    } else {
        index.clamp(pinned, peers.len())
    };
    let at = match peers.get(index) {
        Some(&next) => next,
        None => peers.last().map_or(pos.min(tabs.len()), |&last| last + 1),
    };
    tabs.insert(at, tab);
}

/// Pinned tabs before the others, the order otherwise kept; so it holds in every workspace.
fn group_pinned(tabs: &mut [Tab]) {
    tabs.sort_by_key(|t| !t.pinned);
}

/// Remember a tab that was just closed from place `index` of its workspace. Private
/// tabs and blank ones are not remembered.
fn remember_closed(closed: &mut Vec<Closed>, tab: &Tab, index: usize) {
    if tab.private || !tab.url.starts_with("http") {
        return;
    }
    if closed.len() == CLOSED_MAX {
        closed.remove(0);
    }
    closed.push(Closed {
        url: tab.url.clone(),
        title: tab.title.clone(),
        workspace: tab.workspace,
        pinned: tab.pinned,
        index,
    });
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Side {
    Top,
    Left,
    Right,
}

/// Where the cursor is relative to a collapsed bar.
#[derive(PartialEq, Debug)]
enum Zone {
    /// At the window edge the bar lives on: reveal it.
    Edge,
    /// Over (or just past) the revealed bar: keep it.
    Bar,
    Away,
}

/// `x`,`y` are window coordinates, `w` the window width, `bar` the bar's thickness,
/// `near` how close to the bar's window edge the cursor must come to reveal it.
fn zone(side: Side, x: f64, y: f64, w: f64, bar: f64, near: f64) -> Zone {
    let dist = match side {
        Side::Top => y,
        Side::Left => x,
        Side::Right => w - x,
    };
    match dist {
        d if d < -OVERSHOOT => Zone::Away,
        d if d <= near => Zone::Edge,
        d if d <= bar + LEAVE_MARGIN => Zone::Bar,
        _ => Zone::Away,
    }
}

/// Position `t` (0..1) along a glide that starts fast and settles softly.
fn ease_out(t: f64) -> f64 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(5)
}

struct App {
    // Webviews are declared before the window so they are dropped first.
    tabs: Vec<Tab>,
    floater: Option<WebView>,
    chrome: WebView,
    window: Window,
    proxy: EventLoopProxy<UserEvent>,
    cfg: Config,
    user_agent: Option<String>,
    /// Id of the tab on screen.
    active: Option<u32>,
    /// Id of the workspace on screen.
    workspace: u32,
    /// workspace id -> the tab last used there.
    last_active: HashMap<u32, u32>,
    /// Tabs closed during this run, most recent last.
    closed: Vec<Closed>,
    next_id: u32,
    /// Gap the interface leaves for the page: left, top, right, bottom. `None` until
    /// the interface has laid itself out once.
    insets: Option<[f64; 4]>,
    /// A glide in progress: the gap it started from, and when.
    glide: Option<([f64; 4], Instant)>,
    /// Area the floating bar takes, same form as `insets`.
    float_insets: [f64; 4],
    page_radius: f64,
    /// The floating bar's visible panel (same form as `insets`) and its corner radius.
    panel_insets: [f64; 4],
    panel_radius: f64,
    /// Frosted glass shown behind the floating bar, where the platform has it.
    glass: Option<native::Glass>,
    /// macOS: inset of the window buttons, as last applied.
    lights: [f64; 2],
    settings_open: bool,
    /// A menu or sheet in the interface is using the page area.
    cover: bool,
    /// The collapsed bar is temporarily shown (floating over the page).
    peek: bool,
    bar_busy: bool,
    /// When the cursor left a revealed bar.
    away_since: Option<Instant>,
    /// Development only: keep the bar revealed whatever the cursor does.
    hold_peek: bool,
    /// A newer version, downloaded and waiting for quit (or "Restart to update").
    update: Option<update::Ready>,
    /// What the last update check found, for the settings page.
    update_note: String,
    /// Started with `--private`: every tab is private and nothing is saved.
    private_window: bool,
    started: bool,
    quit: bool,
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
    Rect {
        position: LogicalPosition::new(x, y).into(),
        size: LogicalSize::new(w.max(1.0), h.max(1.0)).into(),
    }
}

fn keys_script(cfg: &Config) -> String {
    let combos: Vec<&String> = cfg.hotkeys.values().collect();
    include_str!("keys.js").replace("__KEYS__", &serde_json::to_string(&combos).unwrap())
}

/// Build a webview running the interface: the main one, or the floating copy.
fn build_ui(
    window: &Window,
    proxy: &EventLoopProxy<UserEvent>,
    cfg: &Config,
    floater: bool,
) -> wry::Result<WebView> {
    let proxy = proxy.clone();
    let webview = WebViewBuilder::new()
        .with_html(include_str!("ui.html"))
        .with_initialization_script(format!("window.__wispFloater = {floater};"))
        .with_initialization_script(keys_script(cfg))
        .with_transparent(floater)
        .with_visible(!floater)
        .with_ipc_handler(move |req| match serde_json::from_str(req.body()) {
            Ok(msg) => drop(proxy.send_event(UserEvent::Ui(msg))),
            Err(e) => eprintln!("wisp: bad UI message: {e}"),
        })
        // The UI webview is privileged; never let it navigate to real content.
        .with_navigation_handler(|url| {
            !["http:", "https:", "file:"]
                .iter()
                .any(|p| url.starts_with(p))
        })
        .build_as_child(window)?;
    adblock::attach(&webview); // it loads site icons; hold it to the same block list
    Ok(webview)
}

/// ponytail: a private window is a second copy of the app started with `--private`,
/// which keeps it fully separate for free but shows a second Dock/taskbar icon.
/// Upgrade path: several windows inside one process.
fn open_private_window() {
    let spawned = std::env::current_exe()
        .and_then(|exe| std::process::Command::new(exe).arg("--private").spawn());
    if let Err(e) = spawned {
        eprintln!("wisp: could not open private window: {e}");
    }
}

impl App {
    fn side(&self) -> Side {
        match self.cfg.layout.mode.as_str() {
            "sidebar-left" => Side::Left,
            "sidebar-right" => Side::Right,
            _ => Side::Top,
        }
    }

    fn size(&self) -> LogicalSize<f64> {
        self.window
            .inner_size()
            .to_logical(self.window.scale_factor())
    }

    fn inset_rect(&self, [l, t, r, b]: [f64; 4]) -> Rect {
        let s = self.size();
        rect(l, t, s.width - l - r, s.height - t - b)
    }

    /// The page gap right now: mid-glide, somewhere between the old and the new one.
    fn page_insets(&self) -> [f64; 4] {
        let target = self.insets.unwrap_or_default();
        let Some((from, start)) = self.glide else {
            return target;
        };
        let k = ease_out(start.elapsed().as_secs_f64() / PAGE_GLIDE.as_secs_f64());
        std::array::from_fn(|i| from[i] + (target[i] - from[i]) * k)
    }

    fn page_visible(&self) -> bool {
        self.insets.is_some() && !self.settings_open && !self.cover
    }

    fn layout_pages(&self) {
        let page = self.inset_rect(self.page_insets());
        for tab in &self.tabs {
            if let Some(w) = &tab.webview {
                let _ = w.set_bounds(page);
                let _ = w.set_visible(self.active == Some(tab.id) && self.page_visible());
            }
        }
    }

    fn layout(&self) {
        let s = self.size();
        let _ = self.chrome.set_bounds(rect(0.0, 0.0, s.width, s.height));
        self.layout_pages();
        if let Some(f) = &self.floater {
            let _ = f.set_bounds(self.inset_rect(self.float_insets));
        }
    }

    /// Advance a page glide by one frame.
    fn tick(&mut self) {
        if let Some((_, start)) = self.glide {
            if start.elapsed() >= PAGE_GLIDE {
                self.glide = None;
            }
            self.layout_pages();
        }
    }

    fn tab(&self, id: u32) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    fn tab_mut(&mut self, id: u32) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    fn active_tab(&self) -> Option<&Tab> {
        self.active.and_then(|id| self.tab(id))
    }

    fn active_webview(&self) -> Option<&WebView> {
        self.active_tab().and_then(|t| t.webview.as_ref())
    }

    /// Ids of the tabs in the workspace on screen, in display order.
    fn workspace_tabs(&self) -> Vec<u32> {
        let ws = self.workspace;
        self.tabs
            .iter()
            .filter(|t| t.workspace == ws)
            .map(|t| t.id)
            .collect()
    }

    /// Both copies of the interface.
    fn uis(&self) -> impl Iterator<Item = &WebView> {
        std::iter::once(&self.chrome).chain(&self.floater)
    }

    /// Push the current state to the UI.
    fn sync(&self) {
        let tabs: Vec<_> = self
            .tabs
            .iter()
            .filter(|t| t.workspace == self.workspace)
            .map(|t| {
                json!({"id": t.id, "title": t.title, "url": t.url, "loading": t.loading,
                       "private": t.private, "pinned": t.pinned})
            })
            .collect();
        let state = json!({
            "tabs": tabs,
            "active": self.active,
            "workspace": self.workspace,
            "config": self.cfg,
            "settings": self.settings_open,
            "peek": self.peek,
            "private": self.private_window,
            "mac": cfg!(target_os = "macos"),
            "glass": self.glass.is_some(),
            "version": env!("CARGO_PKG_VERSION"),
            "update": {
                "supported": update::supported(),
                "ready": self.update.as_ref().map(|u| u.version.as_str()),
                "note": self.update_note,
            },
        });
        let js = format!("window.wisp && wisp.state({state})");
        for ui in self.uis() {
            let _ = ui.evaluate_script(&js);
        }
        let title = self.active_tab().map(|t| t.title.as_str()).unwrap_or("");
        let app = if self.private_window {
            "Wisp (Private)"
        } else {
            "Wisp"
        };
        self.window.set_title(&if title.is_empty() {
            app.to_string()
        } else {
            format!("{title} - {app}")
        });
    }

    fn apply_theme(&self) {
        self.window
            .set_theme(match self.cfg.appearance.theme.as_str() {
                "light" => Some(Theme::Light),
                "dark" => Some(Theme::Dark),
                _ => None,
            });
    }

    fn build_webview(&self, id: u32, url: &str, private: bool) -> Option<WebView> {
        let (p1, p2, p3, p4) = (
            self.proxy.clone(),
            self.proxy.clone(),
            self.proxy.clone(),
            self.proxy.clone(),
        );
        let mut builder = WebViewBuilder::new()
            .with_url(url)
            .with_bounds(self.inset_rect(self.page_insets()))
            .with_visible(self.page_visible())
            .with_incognito(private)
            .with_back_forward_navigation_gestures(true)
            .with_initialization_script(keys_script(&self.cfg))
            // Pages are untrusted: the only message accepted from them is a hotkey press,
            // and only while their tab is the active one (see `PageKey`).
            // ponytail: the active page can still fake a press (e.g. close its own tab)
            // or swallow hotkeys while focused. Upgrade path: native key capture per
            // platform instead of the injected listener.
            .with_ipc_handler(move |req| {
                if let Ok(Msg::Key { combo }) = serde_json::from_str(req.body()) {
                    let _ = p1.send_event(UserEvent::PageKey(id, combo));
                }
            })
            .with_document_title_changed_handler(move |title| {
                let _ = p2.send_event(UserEvent::Title(id, title));
            })
            .with_on_page_load_handler(move |event, url| {
                let started = matches!(event, PageLoadEvent::Started);
                let _ = p3.send_event(UserEvent::Load(id, started, url));
            })
            // target=_blank and window.open become tabs; popups to ad hosts are dropped.
            .with_new_window_req_handler(move |url, _| {
                if !adblock::is_blocked(&url) {
                    let _ = p4.send_event(UserEvent::NewTab(id, url));
                }
                NewWindowResponse::Deny
            });
        if let Some(ua) = &self.user_agent {
            builder = builder.with_user_agent(ua);
        }
        match builder.build_as_child(&self.window) {
            Ok(webview) => {
                adblock::attach(&webview);
                native::round_corners(&webview, self.page_radius);
                let zoom = self.cfg.behavior.default_zoom;
                if zoom != 1.0 {
                    let _ = webview.zoom(zoom);
                }
                // New views stack on top; a floating bar must stay above them.
                if let Some(f) = self.floater.as_ref().filter(|_| self.peek) {
                    if let Some(glass) = &self.glass {
                        glass.restack(&self.window);
                    }
                    native::raise(f, &self.window);
                }
                Some(webview)
            }
            Err(e) => {
                eprintln!("wisp: could not open tab: {e}");
                None
            }
        }
    }

    /// Open a tab in the current workspace and show it.
    fn new_tab(&mut self, url: &str, private: bool) {
        let id = self.next_id;
        self.next_id += 1;
        self.tabs.push(Tab {
            id,
            workspace: self.workspace,
            private: private || self.private_window,
            pinned: false,
            webview: None,
            title: String::new(),
            url: url.to_string(),
            loading: false,
            zoom: self.cfg.behavior.default_zoom,
        });
        self.select(id);
    }

    fn select(&mut self, id: u32) {
        let Some(tab) = self.tab(id) else { return };
        let (ws, url, private, unloaded) = (
            tab.workspace,
            tab.url.clone(),
            tab.private,
            tab.webview.is_none(),
        );
        if unloaded {
            let webview = self.build_webview(id, &url, private);
            if let Some(tab) = self.tab_mut(id) {
                tab.webview = webview;
            }
        }
        self.workspace = ws;
        self.active = Some(id);
        self.last_active.insert(ws, id);
        self.settings_open = false;
        self.layout_pages();
        if let Some(w) = self.active_webview() {
            let _ = w.focus();
        }
        self.sync();
    }

    /// The tab on screen went away (closed or moved): show the nearest remaining tab
    /// of workspace `ws`, counting from list position `pos`, or a fresh one.
    fn show_neighbor(&mut self, ws: u32, pos: usize) {
        let neighbor = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(_, t)| t.workspace == ws)
            .min_by_key(|(i, _)| i.abs_diff(pos))
            .map(|(_, t)| t.id);
        match neighbor {
            Some(next) => self.select(next),
            None => self.open_blank(),
        }
    }

    fn close_tab(&mut self, id: u32) {
        let Some((ws, pos)) = self.remove_tab(id) else {
            return;
        };
        if self.active == Some(id) {
            self.show_neighbor(ws, pos);
        } else {
            self.sync();
        }
        self.save_session();
    }

    // ---- tab basics: reorder, pin, duplicate, close others, reopen ----

    /// Take a tab out of the list, remembering it for `reopen_tab`. Gives its
    /// workspace and the list position it had.
    fn remove_tab(&mut self, id: u32) -> Option<(u32, usize)> {
        let pos = self.tabs.iter().position(|t| t.id == id)?;
        let ws = self.tabs[pos].workspace;
        let index = self.tabs[..pos]
            .iter()
            .filter(|t| t.workspace == ws)
            .count();
        let tab = self.tabs.remove(pos);
        remember_closed(&mut self.closed, &tab, index);
        Some((ws, pos))
    }

    /// Add a tab that is not loaded yet at `index` of its workspace, and show it.
    fn insert_tab(&mut self, mut tab: Tab, index: usize) {
        tab.id = self.next_id;
        self.next_id += 1;
        let id = tab.id;
        self.tabs.push(tab);
        place_tab(&mut self.tabs, id, index);
        self.select(id);
        self.save_session();
    }

    fn reorder_tab(&mut self, id: u32, index: usize) {
        place_tab(&mut self.tabs, id, index);
        self.sync();
        self.save_session();
    }

    fn set_pinned(&mut self, id: u32, pinned: bool) {
        // A private tab is never saved, so pinning it would promise something untrue.
        let Some(tab) = self.tab_mut(id).filter(|t| !t.private) else {
            return;
        };
        tab.pinned = pinned;
        // To the border between the groups: last of the pinned, or first of the rest.
        self.reorder_tab(id, if pinned { usize::MAX } else { 0 });
    }

    /// A copy of the tab, right after it (or first among the unpinned, for a pinned one).
    fn duplicate_tab(&mut self, id: u32) {
        let Some(pos) = self.tabs.iter().position(|t| t.id == id) else {
            return;
        };
        let src = &self.tabs[pos];
        let after = self.tabs[..=pos]
            .iter()
            .filter(|t| t.workspace == src.workspace)
            .count();
        let copy = Tab {
            id: 0, // given by `insert_tab`
            workspace: src.workspace,
            private: src.private,
            pinned: false,
            webview: None,
            title: src.title.clone(),
            url: src.url.clone(),
            loading: false,
            zoom: self.cfg.behavior.default_zoom,
        };
        self.insert_tab(copy, after);
    }

    /// Close every unpinned tab of `keep`'s workspace except `keep` itself.
    fn close_other_tabs(&mut self, keep: u32) {
        let Some(ws) = self.tab(keep).map(|t| t.workspace) else {
            return;
        };
        let others: Vec<u32> = self
            .tabs
            .iter()
            .filter(|t| t.workspace == ws && t.id != keep && !t.pinned)
            .map(|t| t.id)
            .collect();
        for id in others {
            self.remove_tab(id);
        }
        if self.active.is_some_and(|id| self.tab(id).is_none()) {
            self.select(keep);
        } else {
            self.sync();
        }
        self.save_session();
    }

    /// Bring back the tab closed last, in the workspace and place it had.
    fn reopen_tab(&mut self) {
        let Some(closed) = self.closed.pop() else {
            return;
        };
        let known = self.cfg.workspaces.iter().any(|w| w.id == closed.workspace);
        let tab = Tab {
            id: 0, // given by `insert_tab`
            workspace: if known {
                closed.workspace
            } else {
                self.workspace
            },
            private: false,
            pinned: closed.pinned,
            webview: None,
            title: closed.title,
            url: closed.url,
            loading: false,
            zoom: self.cfg.behavior.default_zoom,
        };
        self.insert_tab(tab, closed.index);
    }

    /// The close-tab hotkey: a pinned tab stays open, the hotkey only steps off it.
    fn close_active(&mut self) {
        let Some((id, pinned, ws)) = self.active_tab().map(|t| (t.id, t.pinned, t.workspace))
        else {
            return;
        };
        if !pinned {
            return self.close_tab(id);
        }
        let unpinned = |t: &&Tab| t.workspace == ws && !t.pinned;
        match self.tabs.iter().find(unpinned).map(|t| t.id) {
            Some(next) => self.select(next),
            None => self.open_blank(),
        }
    }

    fn move_tab(&mut self, id: u32, to: u32) {
        let known = self.cfg.workspaces.iter().any(|w| w.id == to);
        let Some(pos) = self.tabs.iter().position(|t| t.id == id) else {
            return;
        };
        let from = self.tabs[pos].workspace;
        if !known || from == to {
            return;
        }
        self.tabs[pos].workspace = to;
        self.last_active.insert(to, id);
        if self.active == Some(id) {
            // It left the workspace on screen; hide it and show what remains here.
            if let Some(w) = &self.tabs[pos].webview {
                let _ = w.set_visible(false);
            }
            self.show_neighbor(from, pos);
        } else {
            self.sync();
        }
        group_pinned(&mut self.tabs); // it joins its group in the new workspace
        self.save_session();
    }

    fn open_blank(&mut self) {
        let url = self.cfg.behavior.new_tab_url.clone();
        self.new_tab(&url, false);
        self.focus_url();
    }

    fn select_workspace(&mut self, ws: u32) {
        if !self.cfg.workspaces.iter().any(|w| w.id == ws) {
            return;
        }
        self.workspace = ws;
        let remembered = self.last_active.get(&ws).copied();
        let target = remembered
            .filter(|&id| self.tab(id).is_some_and(|t| t.workspace == ws))
            .or_else(|| self.workspace_tabs().first().copied());
        match target {
            Some(id) => self.select(id),
            None => self.open_blank(),
        }
        self.save_session();
    }

    /// Move `step` places through `ids`, starting from `current`, wrapping around.
    fn cycle(ids: &[u32], current: Option<u32>, step: isize) -> Option<u32> {
        let n = ids.len() as isize;
        if n == 0 {
            return None;
        }
        let pos = current.and_then(|c| ids.iter().position(|&i| i == c));
        let next = pos.map_or(0, |p| (p as isize + step).rem_euclid(n));
        Some(ids[next as usize])
    }

    /// The floating copy of the interface, created the first time it is needed and
    /// then kept (hidden) so that revealing the bar is instant.
    fn ensure_floater(&mut self) {
        if self.floater.is_none() {
            match build_ui(&self.window, &self.proxy, &self.cfg, true) {
                Ok(f) => self.floater = Some(f),
                Err(e) => eprintln!("wisp: could not create floating bar: {e}"),
            }
        }
    }

    fn set_peek(&mut self, on: bool) {
        let on = on && self.cfg.layout.collapsed;
        self.away_since = None;
        if self.peek == on {
            return;
        }
        self.peek = on;
        if on {
            self.ensure_floater();
            if let Some(f) = &self.floater {
                let _ = f.set_bounds(self.inset_rect(self.float_insets));
                let _ = f.set_visible(true);
                if let Some(glass) = &self.glass {
                    let [l, t, r, b] = self.panel_insets;
                    let s = self.size();
                    let frame = [l, t, s.width - l - r, s.height - t - b];
                    glass.show(&self.window, frame, self.panel_radius);
                }
                native::raise(f, &self.window);
            }
        }
        self.sync(); // the floater slides in, or slides out and reports `FloaterDone`
    }

    /// While the bar is collapsed: reveal it when the cursor reaches its window edge,
    /// hide it again once the cursor has been away for a moment.
    fn poll_cursor(&mut self) {
        if !self.cfg.layout.collapsed || self.settings_open || self.hold_peek {
            return;
        }
        let (Ok(cursor), Ok(origin)) =
            (self.window.cursor_position(), self.window.inner_position())
        else {
            return;
        };
        let scale = self.window.scale_factor();
        let (x, y) = (
            (cursor.x - origin.x as f64) / scale,
            (cursor.y - origin.y as f64) / scale,
        );
        let s = self.size();
        let side = self.side();
        let [l, _, r, b] = self.float_insets;
        let bar = match side {
            Side::Top => s.height - b,
            Side::Left => s.width - r,
            Side::Right => s.width - l,
        };
        // Along the bar's edge the cursor may be a little outside the window; across
        // it, it must be within the window's span.
        let inside = match side {
            Side::Top => (0.0..=s.width).contains(&x),
            _ => (0.0..=s.height).contains(&y),
        };
        let near = self.cfg.layout.hover_zone as f64;
        let zone = if inside {
            zone(side, x, y, s.width, bar, near)
        } else {
            Zone::Away
        };
        if !self.peek {
            if zone == Zone::Edge && self.cfg.layout.hover_expand && !self.cover {
                self.set_peek(true);
            }
        } else if zone != Zone::Away || self.bar_busy {
            self.away_since = None;
        } else if self.away_since.get_or_insert_with(Instant::now).elapsed() >= LEAVE_DELAY {
            self.set_peek(false);
        }
    }

    fn focus_url(&mut self) {
        if self.settings_open {
            self.set_settings_open(false);
        }
        // The interface focuses its address bar, or answers `PeekFocus` if it is hidden.
        let _ = self.chrome.focus();
        let _ = self.chrome.evaluate_script("wisp.focusUrl()");
    }

    fn set_settings_open(&mut self, open: bool) {
        self.settings_open = open;
        self.layout_pages();
        let _ = match self.active_webview() {
            Some(w) if !open => w.focus(),
            _ => self.chrome.focus(),
        };
        self.sync();
    }

    fn action(&mut self, name: &str) {
        match name {
            "new_tab" => return self.open_blank(),
            "new_private_tab" => {
                let url = self.cfg.behavior.new_tab_url.clone();
                self.new_tab(&url, true);
                return self.focus_url();
            }
            "private_window" => return open_private_window(),
            "check_update" => return self.check_update(true),
            "restart_update" => return self.restart_into_update(),
            "licenses" => {
                // The notices as they were for this exact version.
                let notices = concat!(
                    env!("CARGO_PKG_REPOSITORY"),
                    "/blob/v",
                    env!("CARGO_PKG_VERSION"),
                    "/THIRD-PARTY-NOTICES"
                );
                self.new_tab(notices, false);
                return self.set_settings_open(false);
            }
            "next_tab" | "prev_tab" => {
                let step = if name == "next_tab" { 1 } else { -1 };
                if let Some(id) = Self::cycle(&self.workspace_tabs(), self.active, step) {
                    self.select(id);
                }
                return;
            }
            "next_workspace" | "prev_workspace" => {
                let step = if name == "next_workspace" { 1 } else { -1 };
                let ids: Vec<u32> = self.cfg.workspaces.iter().map(|w| w.id).collect();
                if let Some(id) = Self::cycle(&ids, Some(self.workspace), step) {
                    self.select_workspace(id);
                }
                return;
            }
            "toggle_bar" => {
                self.cfg.layout.collapsed = !self.cfg.layout.collapsed;
                self.cfg.save();
                return self.bar_mode_changed();
            }
            "focus_url" => return self.focus_url(),
            "settings" => return self.set_settings_open(!self.settings_open),
            "close_tab" => return self.close_active(),
            "reopen_tab" => return self.reopen_tab(),
            _ => {}
        }
        let home = self.cfg.behavior.homepage.clone();
        let Some(tab) = self.active.and_then(|id| self.tab_mut(id)) else {
            return;
        };
        let Some(w) = &tab.webview else { return };
        let _ = match name {
            "reload" => w.reload(),
            "back" => w.go_back(),
            "forward" => w.go_forward(),
            "home" => w.load_url(&home),
            "zoom_in" | "zoom_out" | "zoom_reset" => {
                tab.zoom = config::clamp_zoom(match name {
                    "zoom_in" => tab.zoom + 0.1,
                    "zoom_out" => tab.zoom - 0.1,
                    _ => 1.0,
                });
                w.zoom(tab.zoom)
            }
            _ => Ok(()),
        };
    }

    /// The bar was collapsed, pinned open, or moved: drop any reveal in progress.
    fn bar_mode_changed(&mut self) {
        self.peek = false;
        self.away_since = None;
        if let Some(f) = &self.floater {
            let _ = f.set_visible(false);
        }
        if let Some(glass) = &self.glass {
            glass.hide();
        }
        if self.cfg.layout.collapsed {
            self.ensure_floater();
        }
        self.sync();
    }

    /// Look for a new version on a background thread; the answer arrives as an event.
    /// `asked` = the user pressed the button (so report back even when nothing is new).
    fn check_update(&mut self, asked: bool) {
        if !update::supported() || self.private_window || self.update.is_some() {
            return;
        }
        if !asked && !self.cfg.behavior.auto_update {
            return;
        }
        if asked {
            self.update_note = "Checking\u{2026}".into();
            self.sync();
        }
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let found = update::fetch().map(|ready| ready.map(Box::new));
            let _ = proxy.send_event(UserEvent::Update(found));
        });
    }

    /// Install the downloaded version now and start it.
    fn restart_into_update(&mut self) {
        let Some(ready) = self.update.take() else {
            return;
        };
        self.save_session();
        match ready.install() {
            Ok(()) => {
                // (On Windows the installer has taken over and this line is never reached.)
                if let Ok(exe) = std::env::current_exe() {
                    let _ = std::process::Command::new(exe).spawn();
                }
                self.quit = true;
            }
            Err(e) => {
                self.update_note = format!("Could not install the update: {e}");
                self.sync();
            }
        }
    }

    fn recompile_adblock(&self) {
        let proxy = self.proxy.clone();
        adblock::compile(adblock::configure(&self.cfg.adblock), move || {
            let _ = proxy.send_event(UserEvent::AdblockReady);
        });
    }

    fn apply_config(&mut self, new: Config) {
        let old = std::mem::replace(&mut self.cfg, new.sanitized());
        self.cfg.save();
        if old.adblock != self.cfg.adblock {
            self.recompile_adblock();
        }
        if old.hotkeys != self.cfg.hotkeys {
            let combos: Vec<&String> = self.cfg.hotkeys.values().collect();
            let js = format!("window.__wispKeys = new Set({})", json!(combos));
            let pages = self.tabs.iter().filter_map(|t| t.webview.as_ref());
            for w in pages.chain(self.uis()) {
                let _ = w.evaluate_script(&js);
            }
        }
        if old.workspaces != self.cfg.workspaces {
            // Tabs of a deleted workspace move to the first one rather than vanish.
            let ids: Vec<u32> = self.cfg.workspaces.iter().map(|w| w.id).collect();
            for tab in &mut self.tabs {
                if !ids.contains(&tab.workspace) {
                    tab.workspace = ids[0];
                }
            }
            group_pinned(&mut self.tabs);
            if !ids.contains(&self.workspace) {
                self.select_workspace(ids[0]);
            }
            self.save_session();
        }
        self.apply_theme();
        if old.layout.collapsed != self.cfg.layout.collapsed
            || old.layout.mode != self.cfg.layout.mode
        {
            self.bar_mode_changed();
        } else {
            self.sync();
        }
    }

    fn start(&mut self) {
        self.started = true;
        if self.cfg.layout.collapsed {
            self.ensure_floater();
        }
        let first_ws = self.cfg.workspaces[0].id;
        self.workspace = first_ws;
        if self.private_window {
            return self.open_blank();
        }
        let session = if self.cfg.behavior.restore_session {
            config::load_session()
        } else {
            Session::default()
        };
        let known = |ws: u32| self.cfg.workspaces.iter().any(|w| w.id == ws);
        let start_ws = Some(session.workspace)
            .filter(|&w| known(w))
            .unwrap_or(first_ws);
        for saved in session.tabs {
            let workspace = Some(saved.workspace)
                .filter(|&w| known(w))
                .unwrap_or(first_ws);
            let id = self.next_id;
            self.next_id += 1;
            if saved.active {
                self.last_active.insert(workspace, id);
            }
            self.tabs.push(Tab {
                id,
                workspace,
                private: false,
                pinned: saved.pinned,
                webview: None,
                title: saved.title,
                url: saved.url,
                loading: false,
                zoom: self.cfg.behavior.default_zoom,
            });
        }
        group_pinned(&mut self.tabs);
        if self.tabs.is_empty() {
            let home = self.cfg.behavior.homepage.clone();
            self.new_tab(&home, false);
        } else {
            self.select_workspace(start_ws);
        }
        // First launch after an update: show what changed.
        if config::updated_from().is_some() {
            let notes = concat!(
                env!("CARGO_PKG_REPOSITORY"),
                "/releases/tag/v",
                env!("CARGO_PKG_VERSION")
            );
            self.new_tab(notes, false);
        }
    }

    fn save_session(&self) {
        if self.private_window || !self.started {
            return;
        }
        let tabs = self
            .tabs
            .iter()
            .filter(|t| !t.private && t.url.starts_with("http"))
            .map(|t| SavedTab {
                url: t.url.clone(),
                title: t.title.clone(),
                workspace: t.workspace,
                pinned: t.pinned,
                active: self.last_active.get(&t.workspace) == Some(&t.id),
            })
            .collect();
        config::save_session(&Session {
            workspace: self.workspace,
            tabs,
        });
    }

    fn handle(&mut self, event: UserEvent) {
        match event {
            UserEvent::Ui(msg) => match msg {
                Msg::Ready => self.sync(),
                Msg::Drag => {
                    let _ = self.window.drag_window();
                }
                Msg::Maximize => self.window.set_maximized(!self.window.is_maximized()),
                Msg::Key { combo } => {
                    if let Some(action) = self.cfg.action_for(&combo).map(str::to_string) {
                        self.action(&action);
                    }
                }
                Msg::Action { name } => self.action(&name),
                Msg::Navigate { url } => {
                    let url = config::resolve_input(&url, &self.cfg.behavior.search_url);
                    if let Some(w) = self.active_webview() {
                        let _ = w.load_url(&url);
                        let _ = w.focus();
                    }
                }
                Msg::SelectTab { id } => self.select(id),
                Msg::CloseTab { id } => self.close_tab(id),
                Msg::MoveTab { id, workspace } => self.move_tab(id, workspace),
                Msg::SelectWorkspace { id } => self.select_workspace(id),
                Msg::ReorderTab { id, index } => self.reorder_tab(id, index),
                Msg::PinTab { id, pinned } => self.set_pinned(id, pinned),
                Msg::DuplicateTab { id } => self.duplicate_tab(id),
                Msg::CloseOtherTabs { id } => self.close_other_tabs(id),
                Msg::PageRect {
                    page,
                    float,
                    radius,
                    panel,
                    panel_radius,
                    lights,
                    animate,
                } => {
                    let sane = |v: &[f64]| v.iter().all(|n| n.is_finite() && *n >= 0.0);
                    if !sane(&page) || !sane(&float) || !sane(&[radius]) {
                        return;
                    }
                    if animate && self.insets.is_some_and(|old| old != page) {
                        self.glide = Some((self.page_insets(), Instant::now()));
                    } else if !animate {
                        self.glide = None;
                    }
                    if self.lights != lights && sane(&lights) {
                        self.lights = lights;
                        #[cfg(target_os = "macos")]
                        {
                            use tao::platform::macos::WindowExtMacOS;
                            // The library takes "extra title bar height", from which the buttons
                            // end up 2px above the value given (measured on macOS 26).
                            // ponytail: may sit 1-2px off on older systems; set frames directly if so.
                            let at = LogicalPosition::new(lights[0], lights[1] + 2.0);
                            self.window.set_traffic_light_inset(at);
                        }
                    }
                    self.insets = Some(page);
                    self.float_insets = float;
                    if sane(&panel) && sane(&[panel_radius]) {
                        (self.panel_insets, self.panel_radius) = (panel, panel_radius);
                    }
                    if self.page_radius != radius {
                        self.page_radius = radius;
                        let pages = self.tabs.iter().filter_map(|t| t.webview.as_ref());
                        pages.for_each(|w| native::round_corners(w, radius));
                    }
                    self.layout();
                }
                Msg::Cover { on } => {
                    self.cover = on;
                    self.layout_pages();
                    if on {
                        let _ = self.chrome.focus();
                    }
                }
                Msg::Busy { on } => self.bar_busy = on,
                Msg::Peek { on } => self.set_peek(on),
                Msg::PeekFocus => {
                    self.set_peek(true);
                    if let Some(f) = self.floater.as_ref().filter(|_| self.peek) {
                        self.bar_busy = true; // until the address bar there loses focus
                        let _ = f.focus();
                        let _ = f.evaluate_script("wisp.focusUrl()");
                    }
                }
                Msg::FloaterDone => {
                    if let Some(f) = self.floater.as_ref().filter(|_| !self.peek) {
                        let _ = f.set_visible(false);
                        if let Some(glass) = &self.glass {
                            glass.hide();
                        }
                    }
                }
                Msg::SaveSettings { config } => self.apply_config(*config),
            },
            UserEvent::PageKey(id, combo) => {
                // A page can only have keyboard focus while it is the one on screen, so a
                // key from a background tab, or while the interface covers the page, is forged.
                if self.active == Some(id) && self.page_visible() {
                    self.handle(UserEvent::Ui(Msg::Key { combo }));
                }
            }
            UserEvent::Title(id, title) => {
                if let Some(tab) = self.tab_mut(id) {
                    tab.title = title;
                    self.sync();
                }
            }
            UserEvent::Load(id, started, url) => {
                if let Some(tab) = self.tab_mut(id) {
                    tab.loading = started;
                    tab.url = url;
                    self.sync();
                    if !started {
                        self.save_session(); // saved as you browse, so a crash loses nothing
                    }
                }
            }
            UserEvent::NewTab(opener, url) => {
                let private = self.tab(opener).is_some_and(|t| t.private);
                self.new_tab(&url, private);
            }
            UserEvent::Update(found) => {
                match found {
                    Ok(Some(ready)) => {
                        self.update_note = format!(
                            "Version {} is ready. It installs when you quit.",
                            ready.version
                        );
                        self.update = Some(*ready);
                    }
                    Ok(None) => self.update_note = "Wisp is up to date.".into(),
                    Err(e) => self.update_note = format!("Could not check for updates: {e}"),
                }
                self.sync();
            }
            UserEvent::AdblockReady => {
                if adblock::REATTACH_AFTER_COMPILE {
                    let pages = self.tabs.iter().filter_map(|t| t.webview.as_ref());
                    pages.chain(self.uis()).for_each(adblock::attach);
                }
                if !self.started {
                    self.start();
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn install_menu() -> muda::Menu {
    // Without an Edit menu, Cmd+C / Cmd+V / Cmd+Q do nothing in macOS webviews.
    use muda::{Menu, PredefinedMenuItem as P, Submenu};
    let sep = P::separator();
    let app = Submenu::with_items(
        "Wisp",
        true,
        &[
            &P::about(None, None),
            &sep,
            &P::hide(None),
            &P::hide_others(None),
            &sep,
            &P::quit(None),
        ],
    );
    let edit = Submenu::with_items(
        "Edit",
        true,
        &[
            &P::undo(None),
            &P::redo(None),
            &sep,
            &P::cut(None),
            &P::copy(None),
            &P::paste(None),
            &P::select_all(None),
        ],
    );
    let window = Submenu::with_items("Window", true, &[&P::minimize(None), &P::fullscreen(None)]);
    let menu = Menu::with_items(&[&app.unwrap(), &edit.unwrap(), &window.unwrap()]).unwrap();
    menu.init_for_nsapp();
    menu
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let private_window = std::env::args().any(|a| a == "--private");
    let cfg = Config::load();
    #[allow(unused_mut)]
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    #[cfg(all(target_os = "macos", debug_assertions))]
    if std::env::var_os("WISP_SHOT").is_some() {
        // Screenshot runs must not take keyboard focus from whoever is at the machine.
        use tao::platform::macos::EventLoopExtMacOS;
        event_loop.set_activate_ignoring_other_apps(false);
    }
    let proxy = event_loop.create_proxy();

    let builder = WindowBuilder::new()
        .with_title("Wisp")
        .with_inner_size(LogicalSize::new(1280.0, 820.0))
        .with_min_inner_size(LogicalSize::new(560.0, 360.0));
    #[cfg(target_os = "macos")]
    let builder = {
        // The interface runs under the title bar; the traffic lights sit on top of it
        // (the interface always keeps the top-left corner for them).
        use tao::platform::macos::WindowBuilderExtMacOS;
        builder
            .with_titlebar_transparent(true)
            .with_fullsize_content_view(true)
            .with_title_hidden(true)
    };
    let window = builder.build(&event_loop)?;
    #[cfg(target_os = "macos")]
    let _menu = install_menu();

    let chrome = build_ui(&window, &proxy, &cfg, false)?;
    let mut app = App {
        tabs: Vec::new(),
        floater: None,
        chrome,
        window,
        proxy,
        cfg,
        user_agent: native::user_agent(),
        active: None,
        workspace: 0,
        last_active: HashMap::new(),
        closed: Vec::new(),
        next_id: 1,
        insets: None,
        glide: None,
        float_insets: [0.0; 4],
        page_radius: 0.0,
        panel_insets: [0.0; 4],
        panel_radius: 0.0,
        glass: native::Glass::new(),
        lights: [0.0; 2],
        settings_open: false,
        cover: false,
        peek: false,
        bar_busy: false,
        away_since: None,
        hold_peek: false,
        update: None,
        update_note: String::new(),
        private_window,
        started: false,
        quit: false,
    };
    app.apply_theme();
    app.layout();
    app.recompile_adblock(); // tabs open once the block list is ready (AdblockReady)

    #[cfg(all(target_os = "macos", debug_assertions))]
    let launched = Instant::now();
    let mut next_update_check = Instant::now() + Duration::from_secs(5);
    event_loop.run(move |event, _, control_flow| {
        match event {
            Event::UserEvent(e) => app.handle(e),
            Event::WindowEvent {
                event: WindowEvent::Resized(_),
                ..
            } => app.layout(),
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => app.quit = true,
            Event::MainEventsCleared => {
                app.tick();
                app.poll_cursor();
                if app.started && Instant::now() >= next_update_check {
                    next_update_check = Instant::now() + UPDATE_EVERY;
                    app.check_update(false);
                }
                // Development: WISP_PEEK=1 reveals the collapsed bar after 3s and holds it;
                // WISP_ACT=<action> runs an action after 3s; WISP_JS=<script> runs in the
                // interface after 4s;
                // WISP_SHOT=/path.png saves a picture of the window after 6s and quits.
                #[cfg(all(target_os = "macos", debug_assertions))]
                {
                    let after = |secs| launched.elapsed() > Duration::from_secs(secs);
                    if std::env::var_os("WISP_PEEK").is_some() && after(3) && !app.hold_peek {
                        app.hold_peek = true;
                        app.set_peek(true);
                    }
                    if let Some(name) = std::env::var("WISP_ACT").ok().filter(|_| after(3)) {
                        // SAFETY: single-threaded here; nothing else reads the environment.
                        unsafe { std::env::remove_var("WISP_ACT") };
                        app.action(&name);
                    }
                    if let Some(js) = std::env::var("WISP_JS").ok().filter(|_| after(4)) {
                        // SAFETY: as above.
                        unsafe { std::env::remove_var("WISP_JS") };
                        let _ = app.chrome.evaluate_script(&js);
                    }
                    if let Some(path) = std::env::var_os("WISP_SHOT").filter(|_| after(6)) {
                        use tao::platform::macos::WindowExtMacOS;
                        let ok = debug_shot::save(app.window.ns_window(), &path.to_string_lossy());
                        eprintln!("wisp: screenshot {}", if ok { "saved" } else { "FAILED" });
                        app.quit = true;
                    }
                }
            }
            Event::LoopDestroyed => {
                app.save_session();
                // The quiet path: a downloaded update goes in as the app closes.
                if let Some(ready) = app.update.take()
                    && let Err(e) = ready.install()
                {
                    eprintln!("wisp: could not install the update: {e}");
                }
            }
            _ => {}
        }
        *control_flow = if app.quit {
            ControlFlow::Exit
        } else if app.glide.is_some() {
            ControlFlow::WaitUntil(Instant::now() + FRAME)
        } else if app.cfg.layout.collapsed || cfg!(debug_assertions) {
            ControlFlow::WaitUntil(Instant::now() + POLL)
        } else {
            ControlFlow::WaitUntil(next_update_check)
        };
    });
}

#[cfg(test)]
mod tests {
    use super::{
        App, CLOSED_MAX, Side, Tab, Zone, ease_out, group_pinned, place_tab, remember_closed, zone,
    };

    fn tab(id: u32, workspace: u32, pinned: bool) -> Tab {
        Tab {
            id,
            workspace,
            private: false,
            pinned,
            webview: None,
            title: String::new(),
            url: format!("https://example.com/{id}"),
            loading: false,
            zoom: 1.0,
        }
    }

    /// The tabs of one workspace, in the order the interface lists them.
    fn order(tabs: &[Tab], workspace: u32) -> Vec<u32> {
        let of = tabs.iter().filter(|t| t.workspace == workspace);
        of.map(|t| t.id).collect()
    }

    #[test]
    fn a_tab_moves_within_its_workspace_only() {
        // Workspace 1 holds 1, 2, 3; workspace 2 holds 10, 11, interleaved in the list.
        let mut tabs = vec![
            tab(1, 1, false),
            tab(10, 2, false),
            tab(2, 1, false),
            tab(11, 2, false),
            tab(3, 1, false),
        ];
        place_tab(&mut tabs, 1, 2); // first to last
        assert_eq!(order(&tabs, 1), [2, 3, 1]);
        place_tab(&mut tabs, 1, 0); // and back
        assert_eq!(order(&tabs, 1), [1, 2, 3]);
        place_tab(&mut tabs, 3, 1);
        assert_eq!(order(&tabs, 1), [1, 3, 2]);
        place_tab(&mut tabs, 2, 99); // past the end = last
        assert_eq!(order(&tabs, 1), [1, 3, 2]);
        place_tab(&mut tabs, 99, 0); // unknown tab: nothing happens
        assert_eq!(order(&tabs, 1), [1, 3, 2]);
        assert_eq!(order(&tabs, 2), [10, 11]); // the other workspace never changed
        // A workspace's only tab stays put wherever it is sent.
        let mut tabs = vec![tab(1, 1, false), tab(10, 2, false), tab(2, 1, false)];
        place_tab(&mut tabs, 10, 7);
        assert_eq!(order(&tabs, 2), [10]);
        assert_eq!(order(&tabs, 1), [1, 2]);
    }

    #[test]
    fn pinned_and_unpinned_tabs_keep_to_their_groups() {
        let mut tabs = vec![
            tab(1, 1, true),
            tab(2, 1, true),
            tab(3, 1, false),
            tab(4, 1, false),
            tab(5, 1, false),
        ];
        place_tab(&mut tabs, 1, 4); // a pinned tab cannot pass the unpinned ones
        assert_eq!(order(&tabs, 1), [2, 1, 3, 4, 5]);
        place_tab(&mut tabs, 5, 0); // an unpinned tab cannot pass the pinned ones
        assert_eq!(order(&tabs, 1), [2, 1, 5, 3, 4]);
        // Pinning: last of the pinned. Unpinning: first of the rest.
        tabs[4].pinned = true; // tab 4
        place_tab(&mut tabs, 4, usize::MAX);
        assert_eq!(order(&tabs, 1), [2, 1, 4, 5, 3]);
        tabs[0].pinned = false; // tab 2
        place_tab(&mut tabs, 2, 0);
        assert_eq!(order(&tabs, 1), [1, 4, 2, 5, 3]);
        assert!(tabs[..2].iter().all(|t| t.pinned) && tabs[2..].iter().all(|t| !t.pinned));

        // A list out of order (a hand-edited session, a tab from another workspace).
        let mut tabs = vec![tab(1, 1, false), tab(2, 2, true), tab(3, 1, true)];
        group_pinned(&mut tabs);
        assert_eq!(order(&tabs, 1), [3, 1]);
        assert_eq!(order(&tabs, 2), [2]);
    }

    #[test]
    fn closed_tabs_are_remembered_newest_last_up_to_a_limit() {
        let mut closed = Vec::new();
        let mut private = tab(1, 1, false);
        private.private = true;
        remember_closed(&mut closed, &private, 0);
        let mut blank = tab(2, 1, false);
        blank.url = "about:blank".into();
        remember_closed(&mut closed, &blank, 0);
        assert!(closed.is_empty()); // neither private tabs nor blank ones

        for id in 0..CLOSED_MAX as u32 + 5 {
            remember_closed(&mut closed, &tab(id, 3, id == 29), id as usize);
        }
        assert_eq!(closed.len(), CLOSED_MAX);
        assert_eq!(closed[0].url, "https://example.com/5"); // the oldest five fell off
        let last = closed.pop().unwrap();
        assert_eq!(last.url, "https://example.com/29");
        assert_eq!((last.workspace, last.index, last.pinned), (3, 29, true));

        // Reopening puts it back where it was.
        let mut tabs = vec![tab(1, 1, false), tab(2, 1, false), tab(3, 1, false)];
        let mut closed = Vec::new();
        remember_closed(&mut closed, &tabs.remove(1), 1);
        let back = closed.pop().unwrap();
        let mut reopened = tab(9, back.workspace, back.pinned);
        reopened.url = back.url;
        tabs.push(reopened);
        place_tab(&mut tabs, 9, back.index);
        assert_eq!(order(&tabs, 1), [1, 9, 3]);
        assert_eq!(tabs[1].url, "https://example.com/2");
    }

    #[test]
    fn cycling_wraps_and_survives_a_missing_current() {
        let ids = [10, 20, 30];
        assert_eq!(App::cycle(&ids, Some(10), 1), Some(20));
        assert_eq!(App::cycle(&ids, Some(30), 1), Some(10));
        assert_eq!(App::cycle(&ids, Some(10), -1), Some(30));
        assert_eq!(App::cycle(&ids, Some(99), 1), Some(10));
        assert_eq!(App::cycle(&ids, None, -1), Some(10));
        assert_eq!(App::cycle(&[], Some(1), 1), None);
    }

    #[test]
    fn cursor_zones_around_a_collapsed_bar() {
        // 1000px-wide window, bar 260px thick once revealed.
        let z = |side, x, y| zone(side, x, y, 1000.0, 260.0, 30.0);
        assert_eq!(z(Side::Right, 995.0, 300.0), Zone::Edge);
        assert_eq!(z(Side::Right, 800.0, 300.0), Zone::Bar);
        assert_eq!(z(Side::Right, 720.0, 300.0), Zone::Bar); // within the leave margin
        assert_eq!(z(Side::Right, 500.0, 300.0), Zone::Away);
        assert_eq!(z(Side::Left, 5.0, 300.0), Zone::Edge);
        assert_eq!(z(Side::Left, 400.0, 300.0), Zone::Away);
        assert_eq!(z(Side::Top, 500.0, 3.0), Zone::Edge);
        assert_eq!(z(Side::Top, 500.0, 600.0), Zone::Away);
        assert_eq!(z(Side::Right, 975.0, 300.0), Zone::Edge); // inside the reveal distance
        assert_eq!(z(Side::Right, 1060.0, 300.0), Zone::Edge); // overshot the window edge
        assert_eq!(z(Side::Right, 1400.0, 300.0), Zone::Away); // long gone
    }

    #[test]
    fn glide_starts_fast_and_lands_exactly() {
        assert_eq!(ease_out(0.0), 0.0);
        assert_eq!(ease_out(1.0), 1.0);
        assert_eq!(ease_out(7.0), 1.0);
        assert!(ease_out(0.25) > 0.6);
    }
}
