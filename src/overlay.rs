//! Menus that float over the page.
//!
//! The interface (`ui.html`) fills the window beneath the tabs' web views, so a menu it
//! draws itself is hidden by the page wherever the two overlap. The overlay is the
//! raised second copy of the interface (the "floater", which also shows a collapsed bar):
//! the main copy describes a menu as plain data (`Menu`), the app places the floater
//! exactly over that menu (plus room for its shadow, so the rest of the page stays
//! clickable), puts frosted glass beneath it where the platform has it, and hands it the
//! data. A choice comes back as `MenuPick { id }` and is passed to the main copy, which
//! holds what each id does.
//!
//! The floater is privileged like the main copy: only the two interface views can send
//! these messages (pages may only send hotkeys, see `build_webview`), its navigation is
//! locked like the main copy's, and it draws menu text as text, never as markup.

use crate::{App, native, rect};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::{Duration, Instant};

/// With the bar not collapsed, the overlay view is released after this long unused,
/// and created again on the next menu.
pub const IDLE_RELEASE: Duration = Duration::from_secs(5 * 60);

/// `IDLE_RELEASE`, or in debug builds `WISP_OVERLAY_IDLE=<seconds>` to watch it happen.
fn idle_release() -> Duration {
    #[cfg(debug_assertions)]
    if let Some(secs) = std::env::var("WISP_OVERLAY_IDLE")
        .ok()
        .and_then(|s| s.parse().ok())
    {
        return Duration::from_secs(secs);
    }
    IDLE_RELEASE
}

/// When an overlay last used at `used` may be released; `None` while it is needed:
/// showing a menu, waiting to show one, or being the collapsed bar.
fn release_at(needed: bool, used: Instant, idle: Duration) -> Option<Instant> {
    (!needed).then_some(used + idle)
}
const MAX_ENTRIES: usize = 64;
const MAX_BUTTONS: usize = 6;
const MAX_TEXT: usize = 400;

/// One row of a menu.
#[derive(Serialize, Deserialize, PartialEq, Debug)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Entry {
    Item(Item),
    /// A hairline between groups.
    Sep,
    /// A small heading.
    Label {
        text: String,
    },
    /// A quiet line of text, such as "No downloads yet".
    Note {
        text: String,
    },
}

fn no(b: &bool) -> bool {
    !*b
}

/// A row you can choose, or a row of small buttons (`buttons`), each chosen by its `id`.
#[derive(Serialize, Deserialize, PartialEq, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct Item {
    pub id: u32,
    pub label: String,
    /// A name from the interface's icon set ("spin" = a spinner).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// `#rrggbb`: tints a dot (shown when there is no icon), e.g. a workspace's color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// The hotkey, already written for display.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// A second, quieter line under the label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub: Option<String>,
    /// A switch, on or off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<bool>,
    #[serde(default, skip_serializing_if = "no")]
    pub checked: bool,
    #[serde(default, skip_serializing_if = "no")]
    pub danger: bool,
    #[serde(default, skip_serializing_if = "no")]
    pub disabled: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub buttons: Vec<Button>,
}

/// A small button inside a row: an icon, or a short text such as "100%".
#[derive(Serialize, Deserialize, PartialEq, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct Button {
    pub id: u32,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "no")]
    pub on: bool,
    #[serde(default, skip_serializing_if = "no")]
    pub disabled: bool,
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub struct Menu {
    pub entries: Vec<Entry>,
    /// The wider size (the downloads list).
    #[serde(default, skip_serializing_if = "no")]
    pub wide: bool,
}

fn icon_ok(icon: &Option<String>) -> bool {
    icon.as_ref()
        .is_none_or(|i| i.len() <= 24 && i.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'))
}

fn text_ok(s: &str) -> bool {
    s.len() <= MAX_TEXT
}

impl Menu {
    /// Small, and every icon a plain name and every color a plain `#rrggbb`: the overlay
    /// draws it, so it gets nothing else.
    pub fn valid(&self) -> bool {
        self.entries.len() <= MAX_ENTRIES
            && self.entries.iter().all(|e| match e {
                Entry::Item(i) => {
                    text_ok(&i.label)
                        && [&i.hint, &i.sub]
                            .iter()
                            .all(|s| s.as_deref().is_none_or(text_ok))
                        && icon_ok(&i.icon)
                        && i.color.as_deref().is_none_or(crate::config::is_hex_color)
                        && i.buttons.len() <= MAX_BUTTONS
                        && i.buttons.iter().all(|b| {
                            text_ok(&b.label)
                                && b.text.as_deref().is_none_or(text_ok)
                                && icon_ok(&b.icon)
                        })
                }
                Entry::Label { text } | Entry::Note { text } => text_ok(text),
                Entry::Sep => true,
            })
    }
}

/// Show `menu` at `rect` ([x, y, width, height] in the window). `origin` is the point
/// inside it that it grows from, `pad` the room kept around it for its shadow, `radius`
/// its corner radius. `again`: the open menu changed (say, the zoom level), not a new one.
#[derive(Deserialize)]
pub struct Show {
    menu: Menu,
    rect: [f64; 4],
    origin: [f64; 2],
    pad: f64,
    radius: f64,
    #[serde(default)]
    again: bool,
    /// Which build of the menu this is; a choice made in an older one is dropped.
    #[serde(default)]
    generation: u32,
}

#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Msg {
    /// The overlay copy has loaded.
    OverlayReady,
    /// A menu is likely soon (the pointer is on a button that opens one): create the view.
    OverlayWarm,
    Menu(Show),
    /// Chosen in the overlay; passed to the main copy, which knows what `id` does.
    MenuPick {
        id: u32,
        #[serde(default)]
        generation: u32,
    },
    /// Close the overlay menu: Escape or a click elsewhere in the overlay, its view lost
    /// focus (a click on the page or anywhere else), or the main copy closed it.
    MenuClose {
        /// Give the keyboard back to the page (Escape).
        #[serde(default)]
        focus: bool,
    },
}

/// The overlay's own state; the view itself is `App::floater`.
pub struct Overlay {
    /// A menu is showing in it.
    pub open: bool,
    /// Its page has loaded (`OverlayReady`) and can be handed a menu.
    pub ready: bool,
    /// A menu asked for before it was ready.
    pending: Option<Show>,
    /// Last opened or closed, for releasing it when unused.
    used: Instant,
}

impl Default for Overlay {
    fn default() -> Self {
        Self {
            open: false,
            ready: false,
            pending: None,
            used: Instant::now(),
        }
    }
}

fn sane(v: &[f64]) -> bool {
    v.iter().all(|n| n.is_finite() && *n >= 0.0)
}

impl App {
    pub(crate) fn overlay_msg(&mut self, msg: Msg) {
        match msg {
            Msg::OverlayReady => {
                self.overlay.ready = true;
                self.sync(); // its theme and state first, then any menu waiting for it
                if let Some(show) = self.overlay.pending.take() {
                    self.show_menu(show);
                }
            }
            Msg::OverlayWarm => {
                self.ensure_floater();
                self.overlay.used = Instant::now();
            }
            Msg::Menu(show) => self.show_menu(show),
            Msg::MenuPick { id, generation } => {
                if self.overlay.open {
                    let js = format!("wisp.menuPick({id}, {generation})");
                    let _ = self.chrome.evaluate_script(&js);
                }
            }
            Msg::MenuClose { focus } => {
                let was = self.overlay.open;
                self.close_menu();
                // Escape or a choice: the keyboard goes back to the page (or the
                // interface). Not when the overlay lost focus: it went where the user clicked.
                if was && focus {
                    let _ = match self.active_webview().filter(|_| self.page_visible()) {
                        Some(w) => w.focus(),
                        None => self.chrome.focus(),
                    };
                }
            }
        }
    }

    fn show_menu(&mut self, show: Show) {
        let ok = show.menu.valid()
            && sane(&show.rect)
            && sane(&show.origin)
            && sane(&[show.pad, show.radius]);
        // The floater is busy being the revealed bar: the main copy draws it itself.
        if !ok || self.peek {
            return self.menu_fallback();
        }
        self.ensure_floater();
        if self.floater.is_none() {
            return self.menu_fallback();
        }
        self.overlay.used = Instant::now();
        if !self.overlay.ready {
            self.overlay.pending = Some(show);
            return;
        }
        let again = show.again && self.overlay.open;
        let ([x, y, w, h], pad) = (show.rect, show.pad);
        let s = self.size();
        let (left, top) = ((x - pad).max(0.0), (y - pad).max(0.0));
        let (right, bottom) = ((x + w + pad).min(s.width), (y + h + pad).min(s.height));
        let js = format!(
            "wisp.menu({})",
            json!({"menu": show.menu, "x": x - left, "y": y - top, "w": w,
                   "origin": show.origin, "again": again, "generation": show.generation})
        );
        let Some(f) = &self.floater else { return };
        let _ = f.evaluate_script(&js);
        let _ = f.set_bounds(rect(left, top, right - left, bottom - top));
        let _ = f.set_visible(true);
        if let Some(glass) = &self.glass {
            glass.show(&self.window, [x, y, w, h], show.radius);
        }
        native::raise(f, &self.window);
        if !again {
            let _ = f.focus(); // for the keyboard; losing it again closes the menu
        }
        self.overlay.open = true;
        self.overlay.used = Instant::now();
    }

    fn menu_fallback(&self) {
        let _ = self.chrome.evaluate_script("wisp.menuFallback()");
    }

    /// Close the overlay menu, if one is open.
    pub(crate) fn close_menu(&mut self) {
        self.overlay.pending = None;
        if !self.overlay.open {
            return;
        }
        self.overlay.open = false;
        self.overlay.used = Instant::now();
        if let Some(f) = &self.floater {
            let _ = f.evaluate_script("wisp.menuHide()");
            if !self.peek {
                let _ = f.set_visible(false);
                if let Some(glass) = &self.glass {
                    glass.hide();
                }
            }
        }
        let _ = self.chrome.evaluate_script("wisp.menuClosed()");
    }

    /// Release the overlay view once it has gone unused for `IDLE_RELEASE`, unless the
    /// collapsed bar needs it. Gives when to look again.
    pub(crate) fn release_overlay(&mut self) -> Option<Instant> {
        self.floater.as_ref()?;
        let o = &self.overlay;
        let needed = self.cfg.layout.collapsed || self.peek || o.open || o.pending.is_some();
        let at = release_at(needed, o.used, idle_release())?;
        if Instant::now() < at {
            return Some(at);
        }
        self.floater = None;
        self.overlay.ready = false;
        #[cfg(debug_assertions)]
        eprintln!("wisp: overlay released after {:?} unused", idle_release());
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    /// A menu as the interface writes it: every kind of entry and field.
    fn sample() -> Value {
        json!({"entries": [
            {"t": "item", "id": 0, "label": "New tab", "icon": "plus", "hint": "⌘T"},
            {"t": "item", "id": 1, "label": "Move to Work", "color": "#6c8cff"},
            {"t": "sep"},
            {"t": "label", "text": "Downloads"},
            {"t": "note", "text": "No downloads yet"},
            {"t": "item", "id": 2, "label": "report.pdf", "icon": "spin", "sub": "example.com · Downloading…",
             "danger": true, "disabled": true, "checked": true},
            {"t": "item", "id": 3, "label": "Block ads", "icon": "shield", "on": false},
            {"t": "item", "id": 4, "label": "Zoom", "icon": "zoom", "buttons": [
                {"id": 5, "label": "Zoom out", "icon": "minus", "disabled": true},
                {"id": 6, "label": "Reset zoom", "text": "110%"},
                {"id": 7, "label": "Dark", "icon": "moon", "on": true}]}
        ], "wide": true})
    }

    #[test]
    fn a_menu_description_survives_the_round_trip() {
        let menu: Menu = serde_json::from_value(sample()).unwrap();
        assert!(menu.valid());
        // What the overlay is handed is exactly what the main copy described.
        assert_eq!(serde_json::to_value(&menu).unwrap(), sample());
        let Entry::Item(zoom) = &menu.entries[7] else {
            panic!()
        };
        assert_eq!(zoom.buttons[1].text.as_deref(), Some("110%"));
        // Defaults are left out again, so an empty menu stays tiny.
        let empty: Menu = serde_json::from_value(json!({"entries": []})).unwrap();
        assert_eq!(serde_json::to_string(&empty).unwrap(), r#"{"entries":[]}"#);
    }

    #[test]
    fn the_overlay_is_released_only_when_unused() {
        let (t, idle) = (Instant::now(), Duration::from_secs(300));
        assert_eq!(release_at(false, t, idle), Some(t + idle));
        // Showing a menu, a menu waiting for the view to load, or the collapsed bar: kept.
        assert_eq!(release_at(true, t, idle), None);
    }

    #[test]
    fn anything_unexpected_is_refused() {
        let bad = |v: Value| serde_json::from_value::<Menu>(v).map(|m| m.valid());
        // Unknown fields (say, raw markup) and unknown kinds do not parse.
        let html = json!({"entries": [{"t": "item", "id": 0, "label": "x", "html": "<img>"}]});
        assert!(bad(html).is_err());
        assert!(bad(json!({"entries": [{"t": "frame", "src": "https://x"}]})).is_err());
        // Icons are plain names, colors plain hex: neither can carry CSS or markup.
        let icon = json!({"entries": [{"t": "item", "id": 0, "label": "x", "icon": "a\"><b"}]});
        assert!(!bad(icon).unwrap());
        let color = json!({"entries": [{"t": "item", "id": 0, "label": "x", "color": "red;x:y"}]});
        assert!(!bad(color).unwrap());
        // Size limits.
        let many = json!({"entries": vec![json!({"t": "sep"}); MAX_ENTRIES + 1]});
        assert!(!bad(many).unwrap());
        let long = json!({"entries": [{"t": "label", "text": "x".repeat(MAX_TEXT + 1)}]});
        assert!(!bad(long).unwrap());
    }
}
