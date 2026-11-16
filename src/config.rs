//! Settings: one JSON file in the per-user config folder, edited from the in-app settings page.

use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::PathBuf};

#[derive(Serialize, Deserialize, Clone, Default, PartialEq)]
#[serde(default)]
pub struct Config {
    pub layout: Layout,
    pub appearance: Appearance,
    pub behavior: Behavior,
    pub adblock: Adblock,
    pub workspaces: Vec<Workspace>,
    /// action name -> key combo, e.g. "new_tab" -> "Mod+T". "Mod" is Cmd on macOS, Ctrl elsewhere.
    pub hotkeys: BTreeMap<String, String>,
}

pub const MODES: [&str; 4] = [
    "sidebar-right",
    "sidebar-left",
    "top-relaxed",
    "top-compact",
];
pub const TEXTURES: [&str; 5] = ["none", "grain", "dots", "lines", "grid"];
/// Everything the user can place in a toolbar area. `spacer` may appear many times.
pub const ITEMS: [&str; 13] = [
    "collapse",
    "back",
    "forward",
    "reload",
    "home",
    "url",
    "workspaces",
    "new_tab",
    "private_tab",
    "private_window",
    "shield",
    "settings",
    "spacer",
];

#[derive(Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct Layout {
    /// One of `MODES`.
    pub mode: String,
    /// The bar is tucked away to a thin strip at the window edge.
    pub collapsed: bool,
    /// While collapsed, moving the mouse onto the strip shows the bar until it leaves.
    pub hover_expand: bool,
    pub sidebar_width: u16,
    pub areas: Areas,
}

/// Which items sit where, in order. The tab list itself is fixed between
/// `before_tabs` and `after_tabs` (above/below it in a sidebar, left/right in a top bar).
#[derive(Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct Areas {
    pub hotbar: Vec<String>,
    pub before_tabs: Vec<String>,
    pub after_tabs: Vec<String>,
}

impl Default for Areas {
    fn default() -> Self {
        let v = |items: &[&str]| items.iter().map(|s| s.to_string()).collect();
        Self {
            hotbar: v(&[
                "back", "forward", "reload", "spacer", "url", "spacer", "shield", "settings",
                "collapse",
            ]),
            before_tabs: v(&[]),
            after_tabs: v(&["workspaces"]),
        }
    }
}

impl Areas {
    /// Keep known items only, each at most once (spacers excepted), and make sure
    /// the settings button exists somewhere so the user cannot lock themselves out.
    fn sanitize(&mut self) {
        let mut seen = std::collections::HashSet::new();
        for area in [
            &mut self.hotbar,
            &mut self.before_tabs,
            &mut self.after_tabs,
        ] {
            area.retain(|item| {
                ITEMS.contains(&item.as_str()) && (item == "spacer" || seen.insert(item.clone()))
            });
        }
        if !seen.contains("settings") {
            self.after_tabs.push("settings".into());
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct Appearance {
    pub theme: String, // "system" | "light" | "dark"
    pub accent: String,
    pub radius: u8,
    pub font_size: u8,
    /// 0-3 colors painted behind the bar; empty = plain theme background.
    pub background: Vec<String>,
    pub gradient_angle: u16,
    /// How strongly the colors cover the theme background, 0-100.
    pub background_opacity: u8,
    /// One of `TEXTURES`.
    pub texture: String,
    pub texture_strength: u8, // 0-100
    pub custom_css: String,
}

#[derive(Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct Behavior {
    pub homepage: String,
    pub new_tab_url: String,
    /// Search URL with `%s` where the query goes.
    pub search_url: String,
    pub restore_session: bool,
    pub default_zoom: f64,
}

#[derive(Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct Adblock {
    pub enabled: bool,
    pub extra_hosts: Vec<String>,
}

/// A named group of tabs the user creates ("Work", "Shopping", ...).
#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub struct Workspace {
    pub id: u32,
    pub name: String,
    pub color: String,
    /// Name of a line icon from the interface's set; empty = a dot in the workspace color.
    #[serde(default)]
    pub icon: String,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            mode: MODES[0].into(),
            collapsed: false,
            hover_expand: true,
            sidebar_width: 220,
            areas: Areas::default(),
        }
    }
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            accent: "#6c8cff".into(),
            radius: 8,
            font_size: 13,
            background: Vec::new(),
            gradient_angle: 160,
            background_opacity: 100,
            texture: "none".into(),
            texture_strength: 30,
            custom_css: String::new(),
        }
    }
}

impl Default for Behavior {
    fn default() -> Self {
        Self {
            homepage: "https://duckduckgo.com".into(),
            new_tab_url: "about:blank".into(),
            search_url: "https://duckduckgo.com/?q=%s".into(),
            restore_session: true,
            default_zoom: 1.0,
        }
    }
}

impl Default for Adblock {
    fn default() -> Self {
        Self {
            enabled: true,
            extra_hosts: Vec::new(),
        }
    }
}

pub fn default_hotkeys() -> BTreeMap<String, String> {
    // Ctrl+Tab: on macOS Ctrl is its own modifier, elsewhere Ctrl is "Mod".
    let ctrl = if cfg!(target_os = "macos") {
        "Ctrl"
    } else {
        "Mod"
    };
    [
        ("new_tab", "Mod+T".to_string()),
        ("new_private_tab", "Mod+Shift+P".to_string()),
        ("private_window", "Mod+Shift+N".to_string()),
        ("close_tab", "Mod+W".to_string()),
        ("next_tab", format!("{ctrl}+Tab")),
        ("prev_tab", format!("{ctrl}+Shift+Tab")),
        ("next_workspace", "Mod+Alt+ArrowRight".to_string()),
        ("prev_workspace", "Mod+Alt+ArrowLeft".to_string()),
        ("toggle_bar", "Mod+Shift+B".to_string()),
        ("focus_url", "Mod+L".to_string()),
        ("reload", "Mod+R".to_string()),
        ("back", "Mod+BracketLeft".to_string()),
        ("forward", "Mod+BracketRight".to_string()),
        ("home", "Mod+Shift+H".to_string()),
        ("zoom_in", "Mod+Equal".to_string()),
        ("zoom_out", "Mod+Minus".to_string()),
        ("zoom_reset", "Mod+0".to_string()),
        ("settings", "Mod+Comma".to_string()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect()
}

/// `#rrggbb` only: these values are placed into the browser UI's CSS.
fn is_hex_color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

impl Config {
    /// Fill in anything missing or out of range so the rest of the app can trust the values.
    pub fn sanitized(mut self) -> Self {
        let known = default_hotkeys();
        self.hotkeys.retain(|action, _| known.contains_key(action));
        for (action, combo) in known {
            self.hotkeys.entry(action).or_insert(combo);
        }
        self.behavior.default_zoom = clamp_zoom(self.behavior.default_zoom);

        let l = &mut self.layout;
        if !MODES.contains(&l.mode.as_str()) {
            l.mode = MODES[0].into();
        }
        l.sidebar_width = l.sidebar_width.clamp(150, 420);
        l.areas.sanitize();

        let a = &mut self.appearance;
        a.font_size = a.font_size.clamp(10, 20);
        a.radius = a.radius.min(20);
        a.gradient_angle %= 360;
        a.texture_strength = a.texture_strength.min(100);
        a.background_opacity = a.background_opacity.min(100);
        if !TEXTURES.contains(&a.texture.as_str()) {
            a.texture = "none".into();
        }
        if !is_hex_color(&a.accent) {
            a.accent = Appearance::default().accent;
        }
        a.background.retain(|c| is_hex_color(c));
        a.background.truncate(3);

        // Workspaces: unique non-zero ids, valid colors, never empty.
        let mut seen = std::collections::HashSet::new();
        self.workspaces.retain(|w| w.id != 0 && seen.insert(w.id));
        for w in &mut self.workspaces {
            if !is_hex_color(&w.color) {
                w.color = Appearance::default().accent;
            }
            if w.name.trim().is_empty() {
                w.name = format!("Workspace {}", w.id);
            }
            // An icon is a short name; anything else (an old emoji, junk) means "none".
            let named = w.icon.len() <= 16 && w.icon.chars().all(|c| c.is_ascii_lowercase());
            if !named {
                w.icon.clear();
            }
        }
        if self.workspaces.is_empty() {
            self.workspaces.push(Workspace {
                id: 1,
                name: "Personal".into(),
                color: Appearance::default().accent,
                icon: String::new(),
            });
        }
        self
    }

    pub fn load() -> Self {
        fs::read_to_string(dir().join("config.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<Config>(&s).ok())
            .unwrap_or_default()
            .sanitized()
    }

    pub fn save(&self) {
        write("config.json", &serde_json::to_string_pretty(self).unwrap());
    }

    pub fn action_for(&self, combo: &str) -> Option<&str> {
        self.hotkeys
            .iter()
            .find(|(_, c)| c.as_str() == combo)
            .map(|(a, _)| a.as_str())
    }
}

pub fn clamp_zoom(z: f64) -> f64 {
    if z.is_finite() {
        z.clamp(0.3, 4.0)
    } else {
        1.0
    }
}

pub fn dir() -> PathBuf {
    let home = std::env::home_dir().unwrap_or_default();
    let var = |k| std::env::var_os(k).map(PathBuf::from);
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Wisp")
    } else if cfg!(windows) {
        var("APPDATA").unwrap_or(home).join("Wisp")
    } else {
        var("XDG_CONFIG_HOME")
            .unwrap_or(home.join(".config"))
            .join("wisp")
    }
}

fn write(name: &str, contents: &str) {
    let _ = fs::create_dir_all(dir());
    if let Err(e) = fs::write(dir().join(name), contents) {
        eprintln!("wisp: could not write {name}: {e}");
    }
}

/// Open tabs, saved as you browse and reopened on the next start.
#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Session {
    pub workspace: u32,
    pub tabs: Vec<SavedTab>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SavedTab {
    pub url: String,
    pub title: String,
    pub workspace: u32,
    pub active: bool,
}

pub fn load_session() -> Session {
    fs::read_to_string(dir().join("session.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_session(session: &Session) {
    write("session.json", &serde_json::to_string(session).unwrap());
}

/// Turn what the user typed in the address bar into a URL: an address, or a search.
pub fn resolve_input(input: &str, search_url: &str) -> String {
    let s = input.trim();
    if s.is_empty() {
        return "about:blank".into();
    }
    let lower = s.to_ascii_lowercase();
    if ["http://", "https://", "file://", "about:"]
        .iter()
        .any(|p| lower.starts_with(p))
    {
        return s.into();
    }
    let host = s.split(['/', ':']).next().unwrap_or("");
    if !s.contains(char::is_whitespace) {
        if host == "localhost" || host.parse::<std::net::Ipv4Addr>().is_ok() {
            return format!("http://{s}");
        }
        if host.contains('.') && !host.starts_with('.') && !host.ends_with('.') {
            return format!("https://{s}");
        }
    }
    let mut q = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                q.push(b as char)
            }
            b' ' => q.push('+'),
            _ => q.push_str(&format!("%{b:02X}")),
        }
    }
    search_url.replace("%s", &q)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_bar_input() {
        let r = |s| resolve_input(s, "https://s.test/?q=%s");
        assert_eq!(r("example.com"), "https://example.com");
        assert_eq!(r("example.com/a?b=1"), "https://example.com/a?b=1");
        assert_eq!(r("https://x.org/p"), "https://x.org/p");
        assert_eq!(r("localhost:3000"), "http://localhost:3000");
        assert_eq!(r("127.0.0.1:8080/x"), "http://127.0.0.1:8080/x");
        assert_eq!(r("rust lang"), "https://s.test/?q=rust+lang");
        assert_eq!(r("what is 1+1?"), "https://s.test/?q=what+is+1%2B1%3F");
        assert_eq!(r("hello"), "https://s.test/?q=hello");
        assert_eq!(
            r("javascript:alert(1)"),
            "https://s.test/?q=javascript%3Aalert%281%29"
        );
        assert_eq!(r("  "), "about:blank");
    }

    #[test]
    fn sanitize_fills_and_clamps() {
        let mut c = Config::default();
        c.hotkeys.insert("bogus".into(), "Mod+X".into());
        c.hotkeys.insert("new_tab".into(), "Mod+N".into());
        c.behavior.default_zoom = 99.0;
        c.layout.mode = "diagonal".into();
        c.appearance.background = vec!["#112233".into(), "red;}*{display:none".into()];
        let ws = |id, name: &str| Workspace {
            id,
            name: name.into(),
            color: "nope".into(),
            icon: if id == 2 { "cart" } else { "\u{1F6D2}" }.into(),
        };
        c.workspaces = vec![ws(2, "Work"), ws(2, "Dup"), ws(0, "Zero"), ws(3, " ")];
        let c = c.sanitized();
        assert!(!c.hotkeys.contains_key("bogus"));
        assert_eq!(c.hotkeys["new_tab"], "Mod+N");
        assert_eq!(c.action_for("Mod+W"), Some("close_tab"));
        assert_eq!(c.behavior.default_zoom, 4.0);
        assert_eq!(c.layout.mode, "sidebar-right");
        assert_eq!(c.appearance.background, ["#112233"]);
        let names: Vec<_> = c.workspaces.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(names, ["Work", "Workspace 3"]);
        assert_eq!(c.workspaces[0].color, "#6c8cff");
        assert_eq!(c.workspaces[0].icon, "cart");
        assert_eq!(c.workspaces[1].icon, "");

        let mut c = Config::default();
        c.layout.areas.hotbar = ["url", "bogus", "spacer", "url", "spacer", "back"]
            .map(String::from)
            .to_vec();
        c.layout.areas.after_tabs = vec!["back".into()];
        let a = c.sanitized().layout.areas;
        assert_eq!(a.hotbar, ["url", "spacer", "spacer", "back"]);
        assert_eq!(a.after_tabs, ["settings"]);
        assert_eq!(Config::default().sanitized().workspaces[0].name, "Personal");
    }
}
