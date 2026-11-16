#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! Wisp: a small browser shell around the operating system's own web engine.
//! One window holds a "chrome" webview (the sidebar or top bar, plus the settings
//! page) and one webview per tab beside it.

mod adblock;
mod config;

use config::{Config, SavedTab, Session};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use tao::{
    dpi::{LogicalPosition, LogicalSize},
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    window::{Theme, Window, WindowBuilder},
};
use wry::{NewWindowResponse, PageLoadEvent, Rect, WebView, WebViewBuilder};

/// Thickness of the strip left at the window edge while the bar is collapsed.
const STRIP: f64 = 6.0;

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
    SelectWorkspace {
        id: u32,
    },
    /// Mouse entered or left the collapsed bar.
    Peek {
        on: bool,
    },
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
}

struct Tab {
    id: u32,
    workspace: u32,
    private: bool,
    /// `None` until the tab is first shown: restored tabs cost nothing until used.
    webview: Option<WebView>,
    title: String,
    url: String,
    loading: bool,
    zoom: f64,
}

#[derive(PartialEq)]
enum Side {
    Top,
    Left,
    Right,
}

struct App {
    // Webviews are declared before the window so they are dropped first.
    tabs: Vec<Tab>,
    chrome: WebView,
    window: Window,
    proxy: EventLoopProxy<UserEvent>,
    cfg: Config,
    /// Id of the tab on screen.
    active: Option<u32>,
    /// Id of the workspace on screen.
    workspace: u32,
    /// workspace id -> the tab last used there.
    last_active: HashMap<u32, u32>,
    next_id: u32,
    settings_open: bool,
    /// The collapsed bar is temporarily shown (mouse over it, or typing an address).
    peek: bool,
    /// Started with `--private`: every tab is private and nothing is saved.
    private_window: bool,
    /// macOS: whether the bar currently extends under the title bar.
    overlay: Option<bool>,
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

    /// Thickness of the bar when shown. Must match the sizes in ui.html.
    fn bar_size(&self) -> f64 {
        match self.cfg.layout.mode.as_str() {
            "top-compact" => 68.0,
            "top-relaxed" => 84.0,
            _ => self.cfg.layout.sidebar_width as f64,
        }
    }

    fn bar_shown(&self) -> bool {
        !self.cfg.layout.collapsed || self.peek
    }

    /// macOS: let the bar run under the title bar (traffic lights sit on top of it).
    /// Only when the bar is on the lights' side; otherwise they would cover the page.
    fn wants_overlay(&self) -> bool {
        cfg!(target_os = "macos") && !self.cfg.layout.collapsed && self.side() != Side::Right
    }

    fn size(&self) -> LogicalSize<f64> {
        self.window
            .inner_size()
            .to_logical(self.window.scale_factor())
    }

    /// (bar, page) rectangles.
    /// ponytail: a peeking bar pushes the page aside rather than floating over it,
    /// so pages re-flow on hover. Upgrade path: raise the bar webview above the tab
    /// webviews (native z-order per platform) and leave the page size alone.
    fn rects(&self) -> (Rect, Rect) {
        let s = self.size();
        let (w, h) = (s.width, s.height);
        let t = if self.bar_shown() {
            self.bar_size()
        } else {
            STRIP
        };
        let (bar, page) = match self.side() {
            Side::Top => (rect(0.0, 0.0, w, t), rect(0.0, t, w, h - t)),
            Side::Left => (rect(0.0, 0.0, t, h), rect(t, 0.0, w - t, h)),
            Side::Right => (rect(w - t, 0.0, t, h), rect(0.0, 0.0, w - t, h)),
        };
        if self.settings_open {
            (rect(0.0, 0.0, w, h), page)
        } else {
            (bar, page)
        }
    }

    fn layout(&self) {
        let (bar, page) = self.rects();
        let _ = self.chrome.set_bounds(bar);
        for tab in &self.tabs {
            if let Some(w) = &tab.webview {
                let _ = w.set_bounds(page);
                let _ = w.set_visible(self.active == Some(tab.id) && !self.settings_open);
            }
        }
    }

    /// `layout`, plus switching the macOS title bar style when the bar moved.
    fn relayout(&mut self) {
        let overlay = self.wants_overlay();
        if self.overlay != Some(overlay) {
            self.overlay = Some(overlay);
            #[cfg(target_os = "macos")]
            {
                use tao::platform::macos::WindowExtMacOS;
                self.window.set_fullsize_content_view(overlay);
                self.window.set_titlebar_transparent(overlay);
            }
        }
        self.layout();
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

    /// Push the current state to the UI.
    fn sync(&self) {
        let tabs: Vec<_> = self
            .tabs
            .iter()
            .filter(|t| t.workspace == self.workspace)
            .map(|t| {
                json!({"id": t.id, "title": t.title, "url": t.url,
                       "loading": t.loading, "private": t.private})
            })
            .collect();
        let state = json!({
            "tabs": tabs,
            "active": self.active,
            "workspace": self.workspace,
            "config": self.cfg,
            "settings": self.settings_open,
            "shown": self.bar_shown() || self.settings_open,
            "peek": self.peek,
            "private": self.private_window,
            "mac": cfg!(target_os = "macos"),
            "lights": self.overlay == Some(true),
        });
        let _ = self
            .chrome
            .evaluate_script(&format!("window.wisp && wisp.state({state})"));
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
        let built = WebViewBuilder::new()
            .with_url(url)
            .with_bounds(self.rects().1)
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
            })
            .build_as_child(&self.window);
        match built {
            Ok(webview) => {
                adblock::attach(&webview);
                let zoom = self.cfg.behavior.default_zoom;
                if zoom != 1.0 {
                    let _ = webview.zoom(zoom);
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
        self.layout();
        if let Some(w) = self.active_webview() {
            let _ = w.focus();
        }
        self.sync();
    }

    fn close_tab(&mut self, id: u32) {
        let Some(pos) = self.tabs.iter().position(|t| t.id == id) else {
            return;
        };
        let ws = self.tabs.remove(pos).workspace;
        if self.active == Some(id) {
            // Show the nearest remaining tab of that workspace, or a fresh one.
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
        } else {
            self.sync();
        }
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

    fn focus_url(&mut self) {
        if self.settings_open {
            self.set_settings_open(false);
        }
        if !self.bar_shown() {
            self.peek = true; // the UI drops the peek again when the address bar loses focus
            self.relayout();
            self.sync();
        }
        let _ = self.chrome.focus();
        let _ = self.chrome.evaluate_script("wisp.focusUrl()");
    }

    fn set_settings_open(&mut self, open: bool) {
        self.settings_open = open;
        self.layout();
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
                self.peek = false;
                self.cfg.save();
                self.relayout();
                return self.sync();
            }
            "focus_url" => return self.focus_url(),
            "settings" => return self.set_settings_open(!self.settings_open),
            "close_tab" => {
                if let Some(id) = self.active {
                    self.close_tab(id);
                }
                return;
            }
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
            for w in pages.chain([&self.chrome]) {
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
            if !ids.contains(&self.workspace) {
                self.select_workspace(ids[0]);
            }
            self.save_session();
        }
        if !self.cfg.layout.collapsed {
            self.peek = false;
        }
        self.apply_theme();
        self.relayout();
        self.sync();
    }

    fn start(&mut self) {
        self.started = true;
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
                webview: None,
                title: saved.title,
                url: saved.url,
                loading: false,
                zoom: self.cfg.behavior.default_zoom,
            });
        }
        if self.tabs.is_empty() {
            let home = self.cfg.behavior.homepage.clone();
            self.new_tab(&home, false);
        } else {
            self.select_workspace(start_ws);
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
                Msg::SelectWorkspace { id } => self.select_workspace(id),
                Msg::Peek { on } => {
                    let on = on && self.cfg.layout.collapsed;
                    if self.peek != on {
                        self.peek = on;
                        self.relayout();
                        self.sync();
                    }
                }
                Msg::SaveSettings { config } => self.apply_config(*config),
            },
            UserEvent::PageKey(id, combo) => {
                // A page can only have keyboard focus while it is the one on screen, so a
                // key from a background tab, or while settings cover the page, is forged.
                if self.active == Some(id) && !self.settings_open {
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
            UserEvent::AdblockReady => {
                if !self.started {
                    self.start();
                } else if adblock::REATTACH_AFTER_COMPILE {
                    let pages = self.tabs.iter().filter_map(|t| t.webview.as_ref());
                    pages.for_each(adblock::attach);
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
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let builder = WindowBuilder::new()
        .with_title("Wisp")
        .with_inner_size(LogicalSize::new(1200.0, 800.0))
        .with_min_inner_size(LogicalSize::new(480.0, 320.0));
    #[cfg(target_os = "macos")]
    let builder = {
        // The window title is never drawn; whether the bar runs under the title bar
        // is switched at runtime in `App::relayout`.
        use tao::platform::macos::WindowBuilderExtMacOS;
        builder.with_title_hidden(true)
    };
    let window = builder.build(&event_loop)?;
    #[cfg(target_os = "macos")]
    let _menu = install_menu();

    let ui_proxy = proxy.clone();
    let chrome = WebViewBuilder::new()
        .with_html(include_str!("ui.html"))
        .with_initialization_script(keys_script(&cfg))
        .with_ipc_handler(move |req| match serde_json::from_str(req.body()) {
            Ok(msg) => drop(ui_proxy.send_event(UserEvent::Ui(msg))),
            Err(e) => eprintln!("wisp: bad UI message: {e}"),
        })
        // The UI webview is privileged; never let it navigate to real content.
        .with_navigation_handler(|url| {
            !["http:", "https:", "file:"]
                .iter()
                .any(|p| url.starts_with(p))
        })
        .build_as_child(&window)?;

    let mut app = App {
        tabs: Vec::new(),
        chrome,
        window,
        proxy,
        cfg,
        active: None,
        workspace: 0,
        last_active: HashMap::new(),
        next_id: 1,
        settings_open: false,
        peek: false,
        private_window,
        overlay: None,
        started: false,
        quit: false,
    };
    app.apply_theme();
    app.relayout();
    app.recompile_adblock(); // tabs open once the block list is ready (AdblockReady)

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
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
            Event::LoopDestroyed => app.save_session(),
            _ => {}
        }
        if app.quit {
            *control_flow = ControlFlow::Exit;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::App;

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
}
