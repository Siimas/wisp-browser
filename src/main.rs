#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! Wisp: a small browser shell around the operating system's own web engine.
//! One window holds a "chrome" webview (tab strip, toolbar, settings page) and one
//! webview per tab below it.

mod adblock;
mod config;

use config::Config;
use serde::Deserialize;
use serde_json::json;
use tao::{
    dpi::{LogicalPosition, LogicalSize},
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    window::{Theme, Window, WindowBuilder},
};
use wry::{NewWindowResponse, PageLoadEvent, Rect, WebView, WebViewBuilder};

/// Messages from the UI (and, for `Key` only, from pages).
#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum Msg {
    Ready,
    Drag,
    Key { combo: String },
    Action { name: String },
    Navigate { url: String },
    SelectTab { id: u32 },
    CloseTab { id: u32 },
    SaveSettings { config: Config },
}

enum UserEvent {
    Ui(Msg),
    /// Hotkey reported by the page in tab `.0`.
    PageKey(u32, String),
    Title(u32, String),
    Load(u32, bool, String),
    NewTab(String),
    AdblockReady,
}

struct Tab {
    id: u32,
    webview: WebView,
    title: String,
    url: String,
    loading: bool,
    zoom: f64,
}

struct App {
    // Webviews are declared before the window so they are dropped first.
    tabs: Vec<Tab>,
    chrome: WebView,
    window: Window,
    proxy: EventLoopProxy<UserEvent>,
    cfg: Config,
    active: usize,
    next_id: u32,
    settings_open: bool,
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

impl App {
    fn chrome_height(&self) -> f64 {
        // Must match --tabs-h + --bar-h in ui.html.
        if self.cfg.appearance.compact {
            68.0
        } else {
            84.0
        }
    }

    fn size(&self) -> LogicalSize<f64> {
        self.window
            .inner_size()
            .to_logical(self.window.scale_factor())
    }

    fn layout(&self) {
        let (s, top) = (self.size(), self.chrome_height());
        let chrome_h = if self.settings_open { s.height } else { top };
        let _ = self.chrome.set_bounds(rect(0.0, 0.0, s.width, chrome_h));
        for (i, tab) in self.tabs.iter().enumerate() {
            let _ = tab
                .webview
                .set_bounds(rect(0.0, top, s.width, s.height - top));
            let _ = tab
                .webview
                .set_visible(i == self.active && !self.settings_open);
        }
    }

    /// Push the current state to the UI.
    fn sync(&self) {
        let tabs: Vec<_> = self
            .tabs
            .iter()
            .map(|t| json!({"id": t.id, "title": t.title, "url": t.url, "loading": t.loading}))
            .collect();
        let active = self.tabs.get(self.active);
        let state = json!({
            "tabs": tabs,
            "active": active.map(|t| t.id),
            "config": self.cfg,
            "settings": self.settings_open,
            "mac": cfg!(target_os = "macos"),
        });
        let _ = self
            .chrome
            .evaluate_script(&format!("window.wisp && wisp.state({state})"));
        let title = active.map(|t| t.title.as_str()).filter(|t| !t.is_empty());
        self.window.set_title(title.unwrap_or("Wisp"));
    }

    fn apply_theme(&self) {
        self.window
            .set_theme(match self.cfg.appearance.theme.as_str() {
                "light" => Some(Theme::Light),
                "dark" => Some(Theme::Dark),
                _ => None,
            });
    }

    fn new_tab(&mut self, url: &str) {
        let id = self.next_id;
        self.next_id += 1;
        let (s, top) = (self.size(), self.chrome_height());
        let (p1, p2, p3, p4) = (
            self.proxy.clone(),
            self.proxy.clone(),
            self.proxy.clone(),
            self.proxy.clone(),
        );
        let built = WebViewBuilder::new()
            .with_url(url)
            .with_bounds(rect(0.0, top, s.width, s.height - top))
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
                    let _ = p4.send_event(UserEvent::NewTab(url));
                }
                NewWindowResponse::Deny
            })
            .build_as_child(&self.window);
        let webview = match built {
            Ok(w) => w,
            Err(e) => return eprintln!("wisp: could not open tab: {e}"),
        };
        adblock::attach(&webview);
        let zoom = self.cfg.behavior.default_zoom;
        if zoom != 1.0 {
            let _ = webview.zoom(zoom);
        }
        self.tabs.push(Tab {
            id,
            webview,
            title: String::new(),
            url: url.to_string(),
            loading: false,
            zoom,
        });
        self.select(self.tabs.len() - 1);
    }

    fn select(&mut self, index: usize) {
        self.active = index.min(self.tabs.len().saturating_sub(1));
        self.settings_open = false;
        self.layout();
        if let Some(tab) = self.tabs.get(self.active) {
            let _ = tab.webview.focus();
        }
        self.sync();
    }

    fn close_tab(&mut self, id: u32) {
        let Some(pos) = self.tabs.iter().position(|t| t.id == id) else {
            return;
        };
        if self.tabs.len() == 1 {
            self.quit = true; // closing the last tab closes the window; it stays in the session
            return;
        }
        self.tabs.remove(pos);
        let next = if pos < self.active {
            self.active - 1
        } else {
            self.active
        };
        self.select(next);
    }

    fn focus_url(&mut self) {
        let _ = self.chrome.focus();
        let _ = self.chrome.evaluate_script("wisp.focusUrl()");
    }

    fn set_settings_open(&mut self, open: bool) {
        self.settings_open = open;
        self.layout();
        let _ = match self.tabs.get(self.active) {
            Some(tab) if !open => tab.webview.focus(),
            _ => self.chrome.focus(),
        };
        self.sync();
    }

    fn action(&mut self, name: &str) {
        let count = self.tabs.len();
        match name {
            "new_tab" => {
                let url = self.cfg.behavior.new_tab_url.clone();
                self.new_tab(&url);
                self.focus_url();
                return;
            }
            "next_tab" if count > 0 => return self.select((self.active + 1) % count),
            "prev_tab" if count > 0 => return self.select((self.active + count - 1) % count),
            "focus_url" => {
                if self.settings_open {
                    self.set_settings_open(false);
                }
                return self.focus_url();
            }
            "settings" => return self.set_settings_open(!self.settings_open),
            _ => {}
        }
        let home = self.cfg.behavior.homepage.clone();
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let w = &tab.webview;
        let _ = match name {
            "close_tab" => {
                let id = tab.id;
                return self.close_tab(id);
            }
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
            for w in self.tabs.iter().map(|t| &t.webview).chain([&self.chrome]) {
                let _ = w.evaluate_script(&js);
            }
        }
        self.apply_theme();
        self.layout();
        self.sync();
    }

    fn start(&mut self) {
        self.started = true;
        let mut urls = if self.cfg.behavior.restore_session {
            config::load_session()
        } else {
            Vec::new()
        };
        if urls.is_empty() {
            urls.push(self.cfg.behavior.homepage.clone());
        }
        for url in urls {
            self.new_tab(&url);
        }
    }

    fn save_session(&self) {
        let urls: Vec<String> = self
            .tabs
            .iter()
            .filter(|t| t.url.starts_with("http"))
            .map(|t| t.url.clone())
            .collect();
        config::save_session(&urls);
    }

    fn tab_mut(&mut self, id: u32) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    fn handle(&mut self, event: UserEvent) {
        match event {
            UserEvent::Ui(msg) => match msg {
                Msg::Ready => self.sync(),
                Msg::Drag => {
                    let _ = self.window.drag_window();
                }
                Msg::Key { combo } => {
                    if let Some(action) = self.cfg.action_for(&combo).map(str::to_string) {
                        self.action(&action);
                    }
                }
                Msg::Action { name } => self.action(&name),
                Msg::Navigate { url } => {
                    let url = config::resolve_input(&url, &self.cfg.behavior.search_url);
                    if let Some(tab) = self.tabs.get(self.active) {
                        let _ = tab.webview.load_url(&url);
                        let _ = tab.webview.focus();
                    }
                }
                Msg::SelectTab { id } => {
                    if let Some(pos) = self.tabs.iter().position(|t| t.id == id) {
                        self.select(pos);
                    }
                }
                Msg::CloseTab { id } => self.close_tab(id),
                Msg::SaveSettings { config } => self.apply_config(config),
            },
            UserEvent::PageKey(id, combo) => {
                // Background tabs cannot have keyboard focus, so a key from one is forged.
                if self.tabs.get(self.active).is_some_and(|t| t.id == id) {
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
            UserEvent::NewTab(url) => self.new_tab(&url),
            UserEvent::AdblockReady => {
                if !self.started {
                    self.start();
                } else if adblock::REATTACH_AFTER_COMPILE {
                    self.tabs.iter().for_each(|t| adblock::attach(&t.webview));
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
    let cfg = Config::load();
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let builder = WindowBuilder::new()
        .with_title("Wisp")
        .with_inner_size(LogicalSize::new(1200.0, 800.0))
        .with_min_inner_size(LogicalSize::new(480.0, 320.0));
    #[cfg(target_os = "macos")]
    let builder = {
        // Tabs live in the title bar area, next to the traffic lights.
        use tao::platform::macos::WindowBuilderExtMacOS;
        builder
            .with_titlebar_transparent(true)
            .with_fullsize_content_view(true)
            .with_title_hidden(true)
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
        active: 0,
        next_id: 1,
        settings_open: false,
        started: false,
        quit: false,
    };
    app.apply_theme();
    app.layout();
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
            Event::LoopDestroyed if app.started => app.save_session(),
            _ => {}
        }
        if app.quit {
            *control_flow = ControlFlow::Exit;
        }
    });
}
