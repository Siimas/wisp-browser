//! The media player: what a page says it is playing (from the injected `media.js`),
//! checked here, and the fixed set of buttons the player can press in that page.
//!
//! A page is untrusted, so its report is cut down to a little plain text, two flags
//! and three numbers before anything else sees it. The tab it came from is the one
//! whose webview delivered it (`App` knows that), never anything the page says.

use serde::Deserialize;
use std::time::Instant;

/// Longest message a page may send at all (bytes), checked before parsing.
pub const MAX_MESSAGE: usize = 4096;
/// Longest title or artist kept (characters).
const MAX_TEXT: usize = 120;
/// Longest media taken seriously: a day. Live streams report no duration (0).
const MAX_DURATION: f64 = 86_400.0;
/// How many reports a tab may send in a burst, and how fast that allowance refills.
const BURST: f64 = 8.0;
const PER_SECOND: f64 = 4.0;

/// A report as the page sent it. Unknown fields are ignored, missing ones defaulted.
#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Report {
    gone: bool,
    playing: bool,
    muted: bool,
    title: String,
    artist: String,
    pos: f64,
    dur: f64,
    rate: f64,
    next: bool,
    prev: bool,
}

/// What a tab is playing, once checked.
#[derive(Clone, Debug, PartialEq)]
pub struct Media {
    pub playing: bool,
    pub muted: bool,
    pub title: String,
    pub artist: String,
    /// Position and duration in seconds (0 = unknown, e.g. a live stream), and speed.
    pub pos: f64,
    pub dur: f64,
    pub rate: f64,
    pub next: bool,
    pub prev: bool,
}

/// Plain text only: no control or formatting characters, spaces collapsed, and short.
/// (The interface also sets it as text, never as markup.)
fn clean(s: &str) -> String {
    let words = s.split(|c: char| c.is_whitespace() || c.is_control() || is_format(c));
    let mut out = words
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if let Some((cut, _)) = out.char_indices().nth(MAX_TEXT) {
        out.truncate(cut);
    }
    out
}

/// Invisible characters that can reorder or hide text (bidi overrides, zero-width).
fn is_format(c: char) -> bool {
    matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2069}' | '\u{feff}')
}

fn num(n: f64, max: f64) -> f64 {
    if n.is_finite() {
        n.clamp(0.0, max)
    } else {
        0.0
    }
}

impl Report {
    /// The checked state, or `None` when the page says its media is gone.
    pub fn check(self) -> Option<Media> {
        if self.gone {
            return None;
        }
        let dur = num(self.dur, MAX_DURATION);
        Some(Media {
            playing: self.playing,
            muted: self.muted,
            title: clean(&self.title),
            artist: clean(&self.artist),
            pos: num(self.pos, dur),
            dur,
            rate: num(self.rate, 16.0),
            next: self.next,
            prev: self.prev,
        })
    }
}

/// Per tab: how many reports it may still send right now.
pub struct Limit {
    tokens: f64,
    at: Instant,
}

impl Default for Limit {
    fn default() -> Self {
        Limit {
            tokens: BURST,
            at: Instant::now(),
        }
    }
}

impl Limit {
    /// Take one report's worth; false = over the limit, drop it.
    pub fn take(&mut self, now: Instant) -> bool {
        let refill = now.saturating_duration_since(self.at).as_secs_f64() * PER_SECOND;
        self.tokens = (self.tokens + refill).min(BURST);
        self.at = now;
        if self.tokens < 1.0 {
            return false;
        }
        self.tokens -= 1.0;
        true
    }
}

/// A tab's media: what it plays now, and what the player needs to know about it.
/// Kept in memory only; nothing here is ever saved, private tab or not.
#[derive(Default)]
pub struct Slot {
    pub now: Option<Media>,
    /// When it last started playing: the player shows the most recent.
    pub started: Option<Instant>,
    /// Closed in the player; it comes back when playback starts again.
    pub dismissed: bool,
    limit: Limit,
}

impl Slot {
    pub fn playing(&self) -> bool {
        self.now.as_ref().is_some_and(|m| m.playing)
    }

    pub fn muted(&self) -> bool {
        self.now.as_ref().is_some_and(|m| m.muted)
    }

    pub fn shown(&self) -> bool {
        self.now.is_some() && !self.dismissed
    }

    /// Take a report from the page, if it is within its limit. True if anything changed.
    pub fn report(&mut self, report: Report) -> bool {
        let now = Instant::now();
        if !self.limit.take(now) {
            return false;
        }
        let new = report.check();
        if new.as_ref().is_some_and(|m| m.playing) && !self.playing() {
            self.started = Some(now);
            self.dismissed = false;
        }
        let changed = self.now != new;
        self.now = new;
        changed
    }

    /// For the interface: the media and the tab it plays in.
    pub fn json(&self, id: u32, tab_title: &str, url: &str, private: bool) -> serde_json::Value {
        let m = self.now.as_ref();
        serde_json::json!({ "id": id, "tab_title": tab_title, "url": url, "private": private,
            "title": m.map(|m| &m.title), "artist": m.map(|m| &m.artist),
            "playing": self.playing(), "muted": self.muted(),
            "pos": m.map(|m| m.pos), "dur": m.map(|m| m.dur), "rate": m.map(|m| m.rate),
            "next": m.is_some_and(|m| m.next), "prev": m.is_some_and(|m| m.prev) })
    }
}

/// The player's buttons.
#[derive(Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Op {
    Play,
    Pause,
    Next,
    Prev,
    Mute,
    Seek,
    /// Hide the player until this tab starts playing again (handled by the app).
    Dismiss,
}

/// The script that presses `op` in the page: built from fixed names and a number held
/// to the media's length, never from anything the page sent.
pub fn script(op: Op, at: f64, media: &Media) -> Option<String> {
    let name = match op {
        Op::Play => "play",
        Op::Pause => "pause",
        Op::Next => "next",
        Op::Prev => "prev",
        Op::Mute => "mute",
        Op::Seek if media.dur > 0.0 => "seek",
        Op::Seek | Op::Dismiss => return None,
    };
    let at = num(at, media.dur);
    Some(format!(
        "window.__wispMedia && window.__wispMedia('{name}', {at})"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn report(json: &str) -> Option<Media> {
        serde_json::from_str::<Report>(json).unwrap().check()
    }

    #[test]
    fn text_is_plain_short_and_single_line() {
        let long = "a".repeat(10_000);
        let m = report(&format!(r#"{{"title":"{long}","artist":"x"}}"#)).unwrap();
        assert_eq!(m.title.chars().count(), MAX_TEXT);
        let json = "{\"title\":\"  <b>Song</b>\\n\\t\\u0000 \\u202eevil\\u200b \",\"artist\":\"\u{e9}\u{1f600}\"}";
        let m = report(json).unwrap();
        // Markup stays as literal text (the interface never parses it); controls go.
        assert_eq!(m.title, "<b>Song</b> evil");
        assert_eq!(m.artist, "é😀");
        // Cut on a character boundary, never inside one.
        let wide = "😀".repeat(500);
        let m = report(&format!(r#"{{"title":"{wide}"}}"#)).unwrap();
        assert_eq!(m.title.chars().count(), MAX_TEXT);
    }

    #[test]
    fn numbers_are_held_to_sane_ranges() {
        let m = report(r#"{"pos":500,"dur":100,"rate":1e9}"#).unwrap();
        assert_eq!((m.pos, m.dur, m.rate), (100.0, 100.0, 16.0));
        let m = report(r#"{"pos":-5,"dur":1e300,"rate":-1}"#).unwrap();
        assert_eq!((m.pos, m.dur, m.rate), (0.0, MAX_DURATION, 0.0));
        // Wrong types or junk fail to parse, so the message is dropped.
        assert!(serde_json::from_str::<Report>(r#"{"pos":"NaN"}"#).is_err());
        assert!(report(r#"{"gone":true,"title":"x"}"#).is_none());
        assert_eq!(report("{}").unwrap().title, "");
    }

    #[test]
    fn a_flood_is_cut_to_the_limit() {
        let (mut l, t0) = (Limit::default(), Instant::now());
        let burst = (0..100).filter(|_| l.take(t0)).count();
        assert_eq!(burst, BURST as usize);
        assert!(!l.take(t0 + Duration::from_millis(100)));
        assert!(l.take(t0 + Duration::from_millis(300))); // refilled a little
        let later = t0 + Duration::from_secs(60);
        assert_eq!((0..100).filter(|_| l.take(later)).count(), BURST as usize);
    }

    #[test]
    fn buttons_become_fixed_scripts() {
        let m = report(r#"{"dur":60}"#).unwrap();
        assert_eq!(
            script(Op::Seek, 1e9, &m).unwrap(),
            "window.__wispMedia && window.__wispMedia('seek', 60)"
        );
        assert!(
            script(Op::Seek, f64::NAN, &m)
                .unwrap()
                .ends_with("('seek', 0)")
        );
        assert!(script(Op::Play, 5.0, &m).unwrap().contains("('play', 5)"));
        let live = report(r#"{"dur":0}"#).unwrap();
        assert!(script(Op::Seek, 5.0, &live).is_none());
        assert!(script(Op::Dismiss, 0.0, &m).is_none());
    }
}
