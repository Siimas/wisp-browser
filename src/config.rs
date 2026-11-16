//! Settings: one JSON file in the per-user config folder, edited from the in-app settings page.

use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::PathBuf};

#[derive(Serialize, Deserialize, Clone, Default, PartialEq)]
#[serde(default)]
pub struct Config {
    pub appearance: Appearance,
    pub behavior: Behavior,
    pub adblock: Adblock,
    /// action name -> key combo, e.g. "new_tab" -> "Mod+T". "Mod" is Cmd on macOS, Ctrl elsewhere.
    pub hotkeys: BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct Appearance {
    pub theme: String, // "system" | "light" | "dark"
    pub accent: String,
    pub compact: bool,
    pub radius: u8,
    pub font_size: u8,
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

impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            accent: "#6c8cff".into(),
            compact: false,
            radius: 8,
            font_size: 13,
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
        ("close_tab", "Mod+W".to_string()),
        ("next_tab", format!("{ctrl}+Tab")),
        ("prev_tab", format!("{ctrl}+Shift+Tab")),
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

impl Config {
    /// Fill in anything missing or out of range so the rest of the app can trust the values.
    pub fn sanitized(mut self) -> Self {
        let known = default_hotkeys();
        self.hotkeys.retain(|action, _| known.contains_key(action));
        for (action, combo) in known {
            self.hotkeys.entry(action).or_insert(combo);
        }
        self.behavior.default_zoom = clamp_zoom(self.behavior.default_zoom);
        self.appearance.font_size = self.appearance.font_size.clamp(10, 20);
        self.appearance.radius = self.appearance.radius.min(20);
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

pub fn load_session() -> Vec<String> {
    fs::read_to_string(dir().join("session.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_session(urls: &[String]) {
    write("session.json", &serde_json::to_string(urls).unwrap());
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
        let c = c.sanitized();
        assert!(!c.hotkeys.contains_key("bogus"));
        assert_eq!(c.hotkeys["new_tab"], "Mod+N");
        assert_eq!(c.action_for("Mod+W"), Some("close_tab"));
        assert_eq!(c.behavior.default_zoom, 4.0);
    }
}
