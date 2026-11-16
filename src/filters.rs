//! Filter lists in the Adblock Plus syntax (EasyList, EasyPrivacy and the like): a
//! parser, a converter to WebKit content rules (macOS) and a small matcher (Windows).
//!
//! Lists come from the network, so every line is untrusted input: whatever this does
//! not fully understand is skipped rather than guessed at, nothing here panics on any
//! input, and the only things a rule can do are block a request or hide elements.

// Request types, as bits.
pub const SCRIPT: u16 = 1;
pub const IMAGE: u16 = 1 << 1;
pub const STYLE: u16 = 1 << 2;
pub const FONT: u16 = 1 << 3;
pub const MEDIA: u16 = 1 << 4;
pub const XHR: u16 = 1 << 5;
pub const FRAME: u16 = 1 << 6;
pub const PING: u16 = 1 << 7;
pub const SOCKET: u16 = 1 << 8;
pub const OTHER: u16 = 1 << 9;
pub const POPUP: u16 = 1 << 10;
pub const DOC: u16 = 1 << 11;
/// What a rule without type options applies to: everything but pages and popups.
pub const DEFAULT: u16 = POPUP - 1;
const ALL: u16 = (DOC << 1) - 1;

/// A network rule: block (or, with `allow`, let through) matching requests.
#[derive(Debug, Default, PartialEq)]
pub struct Net {
    pub allow: bool,
    /// `||`: the pattern starts at the host name or one of its dot-separated parts.
    pub host: bool,
    /// `|` at the start or end: anchored to the start or end of the address.
    pub start: bool,
    pub end: bool,
    /// The pattern between the anchors: literal text, `*` (anything) and `^` (a
    /// separator, or the end of the address). Lowercase unless `case`.
    pub body: String,
    pub case: bool,
    /// `Some(true)`: third-party requests only; `Some(false)`: first-party only.
    pub party: Option<bool>,
    /// Bits from the consts above; 0 = no type options given.
    pub types: u16,
    /// Only on pages of these sites (and their subdomains) / never on these.
    pub domains: Vec<String>,
    pub not_domains: Vec<String>,
}

/// An element-hiding rule: `domains##selector`, or with `allow`, `domains#@#selector`.
#[derive(Debug, PartialEq)]
pub struct Hide {
    pub allow: bool,
    pub selector: String,
    pub domains: Vec<String>,
    pub not_domains: Vec<String>,
}

#[derive(Debug, PartialEq)]
pub enum Rule {
    Net(Net),
    Hide(Hide),
}

/// One line of a list; `None` for comments and for anything not understood.
pub fn parse(line: &str) -> Option<Rule> {
    let line = line.trim();
    if line.is_empty() || line.len() > 4096 || line.starts_with(['!', '[']) {
        return None;
    }
    // `domains##selector` and its variants. A `#` elsewhere is part of an address.
    if let Some(i) = line.find('#') {
        let (doms, rest) = line.split_at(i);
        let domain_like = doms
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".,~*-_".contains(&b));
        if domain_like && rest[1..].starts_with(['#', '@', '?', '$', '%']) {
            return parse_hide(doms, rest).map(Rule::Hide);
        }
    }
    parse_net(line).map(Rule::Net)
}

/// Host names as WebKit wants them: lowercase ASCII, with at least one dot.
pub fn valid_host(h: &str) -> bool {
    h.len() <= 253
        && h.contains('.')
        && !h.starts_with(['.', '-'])
        && !h.ends_with(['.', '-'])
        && h.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
}

/// `a.com,~b.a.com` (or `|`-separated) into (sites, excluded sites). `None` when sites
/// were named but none of them is usable: dropping them would widen the rule to all sites.
fn domains(list: &str, sep: char) -> Option<(Vec<String>, Vec<String>)> {
    let (mut on, mut off, mut dropped) = (Vec::new(), Vec::new(), false);
    for d in list.split(sep).map(str::trim).filter(|d| !d.is_empty()) {
        let (not, d) = match d.strip_prefix('~') {
            Some(d) => (true, d.to_ascii_lowercase()),
            None => (false, d.to_ascii_lowercase()),
        };
        match (valid_host(&d), not) {
            (true, false) => on.push(d),
            (true, true) => off.push(d),
            (false, false) => dropped = true,
            (false, true) => {}
        }
    }
    (!(dropped && on.is_empty())).then_some((on, off))
}

/// Selector syntax that only some blockers understand (procedural filters). Such rules
/// are skipped: WebKit cannot run them, and a half-understood rule could hide too much.
const EXTENDED: [&str; 16] = [
    ":-abp-",
    ":-ext-",
    ":has-text(",
    ":contains(",
    ":xpath(",
    ":upward(",
    ":remove(",
    ":remove-attr(",
    ":remove-class(",
    ":style(",
    ":matches-",
    ":min-text-length(",
    ":watch-attr(",
    ":others(",
    ":if(",
    ":nth-ancestor(",
];

fn parse_hide(doms: &str, rest: &str) -> Option<Hide> {
    let (allow, selector) = if let Some(s) = rest.strip_prefix("##") {
        (false, s)
    } else {
        (true, rest.strip_prefix("#@#")?)
    };
    let selector = selector.trim();
    // `##+js(...)` scriptlets and `##^` HTML filters are not element hiding.
    if !safe_selector(selector) || selector.starts_with(['+', '^']) {
        return None;
    }
    if EXTENDED.iter().any(|x| selector.contains(x)) || doms.contains('*') || hides_page(selector) {
        return None;
    }
    let (domains, not_domains) = domains(doms, ',')?;
    Some(Hide {
        allow,
        selector: selector.to_string(),
        domains,
        not_domains,
    })
}

/// Split at `sep` characters outside brackets, parentheses and quotes.
fn split_top(s: &str, sep: impl Fn(char) -> bool) -> Vec<&str> {
    let (mut out, mut start, mut depth, mut quote) = (Vec::new(), 0, 0i32, None);
    for (i, c) in s.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '(' | '[') => depth += 1,
            (None, ')' | ']') => depth -= 1,
            (None, c) if depth == 0 && sep(c) => {
                out.push(&s[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

/// Attributes nearly every page has somewhere: `[lang]` or `[style]` narrows nothing.
const COMMON_ATTRS: [&str; 18] = [
    "id", "class", "style", "href", "src", "lang", "dir", "title", "role", "name", "type", "width",
    "height", "alt", "rel", "tabindex", "hidden", "content",
];

/// One compound selector (`div.ad[x="1"]:has(...)`): its element name, and whether
/// something in it singles elements out: a class or id, an attribute with a value or
/// one most pages lack, or `:has(...)`. Parts inside `:not(...)` and the like count
/// for nothing, since they can only widen what is hidden.
fn compound(c: &str) -> (String, bool) {
    let element: String = c
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '*')
        .collect::<String>()
        .to_ascii_lowercase();
    let (mut parens, mut attr, mut narrow) = (0, None::<String>, false);
    let lower = c.to_ascii_lowercase();
    for (i, ch) in lower.char_indices() {
        match ch {
            '(' => parens += 1,
            ')' => parens -= 1,
            '.' | '#' if parens == 0 && attr.is_none() => narrow = true,
            ':' if parens == 0 && attr.is_none() && lower[i..].starts_with(":has(") => {
                narrow = true
            }
            '[' if parens == 0 => attr = Some(String::new()),
            ']' => {
                if let Some(name) = attr.take()
                    && !COMMON_ATTRS.contains(&name.trim())
                {
                    narrow = true;
                }
            }
            '=' | '^' | '$' | '*' | '~' | '|' if attr.is_some() => {
                narrow = true; // a value follows
                attr = Some("id".into()); // the rest of the brackets is the value
            }
            c => {
                if let Some(name) = attr.as_mut() {
                    name.push(c);
                }
            }
        }
    }
    let root = lower.starts_with(":root");
    (if root { ":root".into() } else { element }, narrow)
}

/// Would this selector hide the page itself? For each selector of the list: refused
/// when the element it hides is `html`, `body`, `head` or `:root` without a class or
/// id, or when no part of it singles anything out (`*`, `:not(.x)`, `[lang]`,
/// `body > *`). `[id^="ad-"]`, `[data-ad-slot]`, `li.ad > a > *` and `##amp-ad`
/// are fine.
fn hides_page(selector: &str) -> bool {
    split_top(selector, |c| c == ',').into_iter().any(|part| {
        let parts: Vec<(String, bool)> =
            split_top(part.trim(), |c| c.is_whitespace() || ">+~".contains(c))
                .into_iter()
                .filter(|c| !c.is_empty())
                .map(compound)
                .collect();
        let page = |e: &str| ["html", "body", "head", ":root"].contains(&e);
        let named = |c: &str| split_top(c, |ch| ch == '.' || ch == '#').len() > 1;
        let subject = split_top(part.trim(), |c| c.is_whitespace() || ">+~".contains(c))
            .into_iter()
            .rfind(|c| !c.is_empty())
            .unwrap_or("");
        match parts.last() {
            None => true,
            Some((e, _)) if page(e) => !named(subject),
            _ => parts
                .iter()
                .all(|(e, narrow)| !narrow && (e.is_empty() || e == "*" || page(e))),
        }
    })
}

/// A selector that cannot be anything but a selector once it is pasted into a style
/// sheet (`selector { display: none }`): no braces, comments, `@` rules or `;` outside
/// strings, balanced quotes and brackets. This is what stops a list from adding
/// styles of its own (say, a background image that reports each page you visit).
fn safe_selector(s: &str) -> bool {
    if s.is_empty() || s.len() > 1000 || s.contains("/*") || s.contains("*/") {
        return false;
    }
    let (mut quote, mut depth, mut escaped) = (None, 0i32, false);
    for c in s.chars() {
        if c.is_control() {
            return false;
        }
        if escaped {
            escaped = false;
            continue;
        }
        match (quote, c) {
            (_, '\\') => escaped = true,
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '{' | '}' | ';' | '@') => return false,
            (None, '(' | '[') => depth += 1,
            (None, ')' | ']') => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    quote.is_none() && depth == 0 && !escaped
}

fn type_bit(name: &str) -> Option<u16> {
    Some(match name {
        "script" => SCRIPT,
        "image" => IMAGE,
        "stylesheet" | "css" => STYLE,
        "font" => FONT,
        "media" => MEDIA,
        "xmlhttprequest" | "xhr" => XHR,
        "subdocument" | "frame" => FRAME,
        "ping" | "beacon" => PING,
        "websocket" => SOCKET,
        "other" | "object" | "object-subrequest" => OTHER,
        "popup" => POPUP,
        "document" | "doc" => DOC,
        "all" => ALL,
        _ => return None,
    })
}

fn parse_net(line: &str) -> Option<Net> {
    let (allow, rest) = match line.strip_prefix("@@") {
        Some(r) => (true, r),
        None => (false, line),
    };
    // Regular-expression rules (`/.../`) are skipped: WebKit's rule language has no
    // alternation or counted repeats, so most of them cannot be carried over exactly.
    if rest.starts_with('/') && (rest.ends_with('/') || rest.contains("/$")) {
        return None;
    }
    let (pattern, options) = match rest.rfind('$') {
        Some(i) => (&rest[..i], Some(&rest[i + 1..])),
        None => (rest, None),
    };
    let mut n = Net {
        allow,
        ..Net::default()
    };
    let (mut types, mut not_types) = (0u16, 0u16);
    for opt in options.into_iter().flat_map(|o| o.split(',')) {
        let opt = opt.trim().to_ascii_lowercase();
        let (inverse, opt) = match opt.strip_prefix('~') {
            Some(o) => (true, o),
            None => (false, opt.as_str()),
        };
        let (name, value) = opt.split_once('=').unwrap_or((opt, ""));
        match name {
            "third-party" | "3p" => n.party = Some(!inverse),
            "first-party" | "1p" => n.party = Some(inverse),
            "match-case" => n.case = !inverse,
            "domain" | "from" if !inverse => (n.domains, n.not_domains) = domains(value, '|')?,
            "important" => {}
            // Anything else (rewrite, csp, redirect, elemhide, ...) is not understood:
            // skip the whole rule rather than apply part of it.
            _ if !value.is_empty() => return None,
            _ => {
                let bit = type_bit(name)?;
                if inverse {
                    not_types |= bit;
                } else {
                    types |= bit;
                }
            }
        }
    }
    n.types = match (types, not_types) {
        (0, 0) => 0,
        (0, not) => DEFAULT & !not,
        (t, not) => t & !not,
    };
    if (types | not_types) != 0 && n.types == 0 {
        return None;
    }

    let mut p = pattern;
    if let Some(r) = p.strip_prefix("||") {
        (n.host, p) = (true, r);
    } else if let Some(r) = p.strip_prefix('|') {
        (n.start, p) = (true, r);
    }
    if let Some(r) = p.strip_suffix('|') {
        (n.end, p) = (true, r);
    }
    if !p.is_ascii() || p.bytes().any(|b| b <= b' ') {
        return None;
    }
    // A leading `*` lifts the start anchor, a trailing one the end anchor.
    let trimmed = p.trim_start_matches('*');
    if trimmed.len() != p.len() {
        (n.host, n.start) = (false, false);
    }
    let body = trimmed.trim_end_matches('*');
    if body.len() != trimmed.len() {
        n.end = false;
    }
    if n.domains.is_empty() && too_broad(&n, body) {
        return None;
    }
    n.body = if n.case {
        body.to_string()
    } else {
        body.to_ascii_lowercase()
    };
    Some(n)
}

/// Second levels under two-letter country domains that are public suffixes ("co.uk").
const SECOND_LEVELS: [&str; 10] = [
    "co", "com", "net", "org", "gov", "edu", "ac", "ne", "or", "go",
];

/// Address words too common to single anything out.
const COMMON: [&str; 20] = [
    "http", "https", "www", "ws", "wss", "com", "net", "org", "html", "htm", "php", "js", "css",
    "png", "jpg", "gif", "svg", "json", "xml", "asp",
];

/// A rule with no site to limit it that would hit nearly every address: a typo or a
/// hostile list, not a filter. Host patterns must name a site (`host_too_broad`); any
/// other pattern needs four characters besides wildcards and a leading scheme, and
/// either an uncommon word of three letters or a path or query (`/`, `?`, `=`, `&`)
/// with five characters. So `*`, `://`, `|http`, `.js`, `.com^` and `.co.uk^` are
/// refused, `/ut.js?cb=` is not. Block rules for pings only are let through: no page
/// breaks without them.
fn too_broad(n: &Net, body: &str) -> bool {
    if n.types == PING && !n.allow {
        return false;
    }
    if n.host {
        return host_too_broad(body);
    }
    let mut rest = body.trim_start_matches(['*', '|']);
    for scheme in ["https", "http", "wss", "ws", "://", ":", "//"] {
        rest = rest.strip_prefix(scheme).unwrap_or(rest);
    }
    if n.start && rest.len() != body.len() {
        return host_too_broad(rest); // `|https://host...`: a host, like `||host`
    }
    let literal = rest.bytes().filter(|b| !b"*^|".contains(b)).count();
    let word = rest
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|w| w.len() >= 3 && !COMMON.contains(&w.to_ascii_lowercase().as_str()));
    let path = literal >= 5 && rest.contains(['/', '?', '=', '&']);
    literal < 4 || !(word || path)
}

/// The host at the start of a pattern must name a site: one of its labels other than
/// the top-level domain (and a public second level, "co.uk") is written out, in full
/// or with three letters beside its wildcard. One label is accepted only followed by
/// a dot ("any top-level domain") and four letters long. So `||com^`, `||co.uk^`,
/// `||*.*^` and `||a*.com^` are refused; `||t.co^` and `||mkto-*.com^` are not.
fn host_too_broad(pattern: &str) -> bool {
    let host = pattern
        .split(['/', '^', ':', '?', '|'])
        .next()
        .unwrap_or("");
    let labels: Vec<&str> = host.split('.').filter(|l| !l.is_empty()).collect();
    let letters = |l: &str| l.bytes().filter(|b| *b != b'*').count();
    let names: &[&str] = match labels.as_slice() {
        [] => return true,
        [one] => return !(host.ends_with('.') && !one.contains('*') && letters(one) >= 4),
        [names @ .., sld, tld] if tld.len() == 2 && SECOND_LEVELS.contains(sld) => names,
        [names @ .., _tld] => names,
    };
    !names.iter().any(|l| !l.contains('*') || letters(l) >= 3)
}

/// The host name, when the rule is exactly `||host^` (or `||host`).
fn pure_host(n: &Net) -> Option<&str> {
    let h = n.body.strip_suffix('^').unwrap_or(&n.body);
    (n.host && !n.end && valid_host(h)).then_some(h)
}

// ---------------------------------------------------------------- WebKit (macOS)

#[cfg(any(target_os = "macos", test))]
pub mod webkit {
    use super::*;
    use serde_json::{Value, json};
    use std::collections::BTreeMap;

    /// The rule's address pattern in WebKit's regular-expression subset (no alternation:
    /// "a separator or the end" is written as an optional group running to the end).
    fn url_filter(n: &Net) -> String {
        let mut s = String::with_capacity(n.body.len() + 24);
        if n.host {
            // The host, or a part of it after a dot or a user name (`user@host`).
            s.push_str("^[^:]+://+([^/]*[.@])?");
        } else if n.start {
            s.push('^');
        }
        let last = n.body.len().wrapping_sub(1);
        for (i, c) in n.body.chars().enumerate() {
            match c {
                '*' => s.push_str(".*"),
                // After a host name the next character is always `/` or `:` (a port).
                '^' if i == last && !n.end && pure_host(n).is_some() => s.push_str("[/:]"),
                '^' if i == last && !n.end => s.push_str("([^a-zA-Z0-9_.%-].*)?$"),
                '^' => s.push_str("[^a-zA-Z0-9_.%-]"),
                '.' | '?' | '+' | '(' | ')' | '[' | ']' | '{' | '}' | '\\' | '$' | '|' => {
                    s.push('\\');
                    s.push(c);
                }
                c => s.push(c),
            }
        }
        if n.end {
            s.push('$');
        }
        if s.is_empty() {
            s.push_str(".*");
        }
        s
    }

    fn wk_types(types: u16) -> Vec<&'static str> {
        let map: [(u16, &[&str]); 9] = [
            (SCRIPT, &["script"]),
            (IMAGE, &["image", "svg-document"]),
            (STYLE, &["style-sheet"]),
            (FONT, &["font"]),
            (MEDIA, &["media"]),
            (XHR | SOCKET | OTHER, &["raw"]),
            (FRAME | DOC, &["document"]),
            (PING, &["ping"]),
            (POPUP, &["popup"]),
        ];
        let mut out: Vec<&str> = Vec::new();
        for (bits, names) in map {
            if types & bits != 0 {
                out.extend(names);
            }
        }
        out
    }

    fn star(domains: &[String]) -> Value {
        domains.iter().map(|d| format!("*{d}")).collect()
    }

    /// WebKit takes either sites to apply to or sites to skip, not both: the first wins.
    fn add_domains(trigger: &mut Value, on: &[String], off: &[String]) {
        if !on.is_empty() {
            trigger["if-domain"] = star(on);
        } else if !off.is_empty() {
            trigger["unless-domain"] = star(off);
        }
    }

    fn net_rule(n: &Net) -> Option<Value> {
        if n.allow && n.types & DOC != 0 {
            // `@@||site^$document`: no blocking at all on that site's pages.
            let site = pure_host(n)?;
            let trigger = json!({"url-filter": ".*", "if-domain": [format!("*{site}")]});
            return Some(json!({"trigger": trigger, "action": {"type": "ignore-previous-rules"}}));
        }
        let mut trigger = json!({"url-filter": url_filter(n)});
        if n.case {
            trigger["url-filter-is-case-sensitive"] = json!(true);
        }
        // A plain `||host^` also stops frames and popups from that host; a path pattern
        // without types must not stop the page you opened (ABP semantics).
        let types = match n.types {
            0 if pure_host(n).is_some() => 0,
            0 => DEFAULT & !FRAME,
            t => t,
        };
        if types != 0 {
            trigger["resource-type"] = json!(wk_types(types));
        }
        if let Some(third) = n.party {
            trigger["load-type"] = json!([if third { "third-party" } else { "first-party" }]);
        }
        add_domains(&mut trigger, &n.domains, &n.not_domains);
        let action = if n.allow {
            "ignore-previous-rules"
        } else {
            "block"
        };
        Some(json!({"trigger": trigger, "action": {"type": action}}))
    }

    fn hide_rules(selectors: &[String], on: &[String], off: &[String], out: &mut Vec<Value>) {
        // Selectors go in groups (fewer rules); WebKit skips a rule whose selector it
        // cannot parse, so the groups are kept small to lose little to one bad entry.
        for group in selectors.chunks(64) {
            let mut trigger = json!({"url-filter": ".*"});
            add_domains(&mut trigger, on, off);
            let action = json!({"type": "css-display-none", "selector": group.join(", ")});
            out.push(json!({"trigger": trigger, "action": action}));
        }
    }

    /// Convert a list into WebKit content-rule JSON, split into parts of at most `max`
    /// rules (WebKit's limit per rule list). Each part ends with the list's exceptions,
    /// since an exception only reaches rules before it in the same rule list.
    pub fn to_webkit(rules: impl Iterator<Item = Rule>, max: usize) -> Vec<String> {
        let (mut generic, mut blocks, mut allows) = (Vec::new(), Vec::new(), Vec::new());
        let mut specific: BTreeMap<(Vec<String>, Vec<String>), Vec<String>> = BTreeMap::new();
        let mut hide_off: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut hidden = std::collections::HashSet::new();
        for rule in rules {
            match rule {
                Rule::Net(n) => {
                    if let Some(r) = net_rule(&n) {
                        (if n.allow { &mut allows } else { &mut blocks }).push(r);
                    }
                }
                Rule::Hide(h) if h.allow => {
                    hide_off.entry(h.selector).or_default().extend(h.domains)
                }
                Rule::Hide(h) if h.domains.is_empty() && h.not_domains.is_empty() => {
                    if hidden.insert(h.selector.clone()) {
                        generic.push(h.selector);
                    }
                }
                Rule::Hide(h) => specific
                    .entry((h.domains, h.not_domains))
                    .or_default()
                    .push(h.selector),
            }
        }
        // A generic rule with exceptions applies everywhere except the excepted sites;
        // an exception with no sites turns the rule off.
        let mut everywhere = Vec::new();
        for sel in generic {
            match hide_off.get(&sel) {
                None => everywhere.push(sel),
                Some(sites) if sites.is_empty() => {}
                Some(sites) => specific
                    .entry((Vec::new(), sites.clone()))
                    .or_default()
                    .push(sel),
            }
        }
        // Generic hiding first: WebKit turns leading `.*` hide rules into one style sheet
        // shared by every page, instead of matching them per page.
        let mut head = Vec::new();
        hide_rules(&everywhere, &[], &[], &mut head);
        head.append(&mut blocks);
        for ((on, off), sels) in &specific {
            hide_rules(sels, on, off, &mut head);
        }
        allows.truncate(max / 2); // a list that is mostly exceptions is not a block list
        let room = max - allows.len();
        head.chunks(room)
            .map(|part| {
                let mut v = part.to_vec();
                v.extend(allows.iter().cloned());
                Value::Array(v).to_string()
            })
            .collect()
    }
}

// ---------------------------------------------------------------- matcher (Windows)

/// Request matching without the engine's help, for WebView2's request filter.
/// ponytail: no element hiding on this path (WebView2 has no content-rule engine);
/// upgrade path: inject the hide selectors as a style sheet per document.
#[cfg(any(windows, test))]
pub mod matcher {
    use super::*;
    use std::collections::{HashMap, HashSet};

    #[derive(Default)]
    pub struct Matcher {
        /// Plain `||host^` rules, by far the most common: a set lookup per parent domain.
        hosts: HashSet<Box<str>>,
        /// Sites where nothing is blocked (`@@||site^$document`).
        sites: HashSet<Box<str>>,
        block: Index,
        allow: Index,
    }

    /// Rules filed under one word of their pattern, so a request is only tested
    /// against rules sharing a word with its address.
    #[derive(Default)]
    struct Index {
        by_token: HashMap<Box<str>, Vec<Net>>,
        rest: Vec<Net>,
    }

    fn token_char(b: u8) -> bool {
        b.is_ascii_alphanumeric() || b == b'%'
    }

    /// The longest word of the pattern that must appear whole in a matching address.
    fn token(n: &Net) -> Option<String> {
        let b = n.body.as_bytes();
        let (mut best, mut i) = (None::<(usize, usize)>, 0);
        while i < b.len() {
            if !token_char(b[i]) {
                i += 1;
                continue;
            }
            let s = i;
            while i < b.len() && token_char(b[i]) {
                i += 1;
            }
            let left = if s == 0 {
                n.host || n.start
            } else {
                b[s - 1] != b'*'
            };
            let right = if i == b.len() { n.end } else { b[i] != b'*' };
            if left && right && i - s >= 2 && best.is_none_or(|(a, z)| i - s > z - a) {
                best = Some((s, i));
            }
        }
        best.map(|(a, z)| n.body[a..z].to_ascii_lowercase())
    }

    impl Index {
        fn add(&mut self, n: Net) {
            match token(&n) {
                Some(t) => self.by_token.entry(t.into()).or_default().push(n),
                None => self.rest.push(n),
            }
        }

        fn find(&self, r: &Req) -> bool {
            let words = r
                .lower
                .split(|c: char| !token_char(c as u8) || !c.is_ascii());
            let tokened = words
                .filter(|w| !w.is_empty())
                .filter_map(|w| self.by_token.get(w))
                .flatten();
            tokened.chain(&self.rest).any(|n| matches(n, r))
        }
    }

    struct Req<'a> {
        url: &'a str,
        lower: &'a str,
        host: (usize, usize),
        page: Option<&'a str>,
        third: bool,
        kind: u16,
    }

    fn under(host: &str, site: &str) -> bool {
        host == site || host.strip_suffix(site).is_some_and(|h| h.ends_with('.'))
    }

    fn host_listed(set: &HashSet<Box<str>>, host: &str) -> bool {
        let mut h = host;
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

    /// The registrable part of a host name ("news.bbc.co.uk" -> "bbc.co.uk").
    /// ponytail: no Public Suffix List; two-letter country domains with a common
    /// second level ("co.uk", "com.au") are recognised, which covers most sites.
    pub fn site_of(host: &str) -> &str {
        let labels: Vec<&str> = host.rsplitn(4, '.').collect();
        let keep = match labels.as_slice() {
            [tld, sld, ..] if tld.len() == 2 && SECOND_LEVELS.contains(sld) => 3,
            _ => 2,
        };
        match host.rmatch_indices('.').nth(keep - 1) {
            Some((i, _)) => &host[i + 1..],
            None => host,
        }
    }

    fn matches(n: &Net, r: &Req) -> bool {
        let types = match n.types {
            0 if n.host => ALL,
            0 => DEFAULT,
            t => t,
        };
        if types & r.kind == 0 {
            return false;
        }
        if let Some(third) = n.party
            && (r.page.is_none() || third != r.third)
        {
            return false;
        }
        if !n.domains.is_empty()
            && !r
                .page
                .is_some_and(|p| n.domains.iter().any(|d| under(p, d)))
        {
            return false;
        }
        if r.page
            .is_some_and(|p| n.not_domains.iter().any(|d| under(p, d)))
        {
            return false;
        }
        let url = if n.case { r.url } else { r.lower }.as_bytes();
        let p = n.body.as_bytes();
        if n.host {
            let (hs, he) = r.host;
            (hs..he)
                .filter(|&i| i == hs || url[i - 1] == b'.')
                .any(|i| glob(p, &url[i..], n.end, false))
        } else {
            glob(p, url, n.end, !n.start)
        }
    }

    fn separator(c: u8) -> bool {
        !(c.is_ascii_alphanumeric() || b"_-.%".contains(&c))
    }

    /// Does `p` (literal bytes, `*`, `^`) match the start of `s` (all of it when `end`,
    /// anywhere in it when `float`)? Linear backtracking over the last `*`, so a
    /// pattern full of stars costs at most len(p) * len(s).
    pub(super) fn glob(p: &[u8], s: &[u8], end: bool, float: bool) -> bool {
        let (mut pi, mut si) = (0, 0);
        let mut back = float.then_some((0, 0));
        loop {
            if pi == p.len() && (!end || si == s.len()) {
                return true;
            }
            if pi < p.len() {
                let c = p[pi];
                if si < s.len() && (c == b'^' && separator(s[si]) || c == s[si] && c != b'*') {
                    pi += 1;
                    si += 1;
                    continue;
                }
                if c == b'*' {
                    back = Some((pi + 1, si));
                    pi += 1;
                    continue;
                }
                if c == b'^' && si == s.len() {
                    pi += 1; // a separator also matches the end of the address
                    continue;
                }
            }
            match back {
                Some((bp, bs)) if bs < s.len() => {
                    back = Some((bp, bs + 1));
                    (pi, si) = (bp, bs + 1);
                }
                _ => return false,
            }
        }
    }

    /// Byte range of the host name in a URL (same rules as `adblock::host_of`).
    fn host_range(url: &str) -> Option<(usize, usize)> {
        let start = url.find("://")? + 3;
        let rest = &url[start..];
        let auth_end = rest.find(['/', '\\', '?', '#']).unwrap_or(rest.len());
        let auth = &rest[..auth_end];
        let hs = auth.rfind('@').map_or(0, |i| i + 1);
        let he = auth[hs..].find(':').map_or(auth.len(), |i| hs + i);
        (he > hs).then_some((start + hs, start + he))
    }

    impl Matcher {
        pub fn add(&mut self, n: Net) {
            if n.allow {
                if n.types & DOC != 0 {
                    if let Some(site) = pure_host(&n) {
                        self.sites.insert(site.into());
                    }
                } else {
                    self.allow.add(n);
                }
                return;
            }
            let plain = n.party.is_none() && n.types == 0 && n.domains.is_empty() && !n.case;
            match pure_host(&n) {
                Some(h) if plain && n.not_domains.is_empty() => {
                    self.hosts.insert(h.into());
                }
                _ => self.block.add(n),
            }
        }

        /// Should a request for `url` of type `kind`, made by the page at `page`, be blocked?
        pub fn blocks(&self, url: &str, page: Option<&str>, kind: u16) -> bool {
            let lower = url.to_ascii_lowercase();
            let Some(host) = host_range(&lower) else {
                return false;
            };
            let page_lower = page.map(str::to_ascii_lowercase);
            let page_host = page_lower
                .as_deref()
                .and_then(|p| host_range(p).map(|(a, z)| &p[a..z]));
            if page_host.is_some_and(|p| host_listed(&self.sites, p)) {
                return false;
            }
            let req_host = &lower[host.0..host.1];
            let r = Req {
                url,
                third: page_host.is_some_and(|p| site_of(p) != site_of(req_host)),
                page: page_host,
                host,
                kind,
                lower: &lower,
            };
            (host_listed(&self.hosts, req_host) || self.block.find(&r)) && !self.allow.find(&r)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::matcher::{Matcher, glob, site_of};
    use super::webkit::to_webkit;
    use super::*;
    use serde_json::{Value, json};

    fn net(line: &str) -> Net {
        match parse(line) {
            Some(Rule::Net(n)) => n,
            other => panic!("{line}: {other:?}"),
        }
    }

    #[test]
    fn parses_network_rules_and_options() {
        let n = net("||ads.example.com^$third-party,script,domain=a.com|~b.a.com");
        assert!(n.host && !n.allow && n.party == Some(true));
        assert_eq!(n.body, "ads.example.com^");
        assert_eq!(n.types, SCRIPT);
        assert_eq!(
            (n.domains, n.not_domains),
            (vec!["a.com".into()], vec!["b.a.com".into()])
        );
        let n = net("@@|https://cdn.Example.com/ads.js|$~third-party,~image");
        assert!(n.allow && n.start && n.end && n.party == Some(false));
        assert_eq!(n.body, "https://cdn.example.com/ads.js");
        assert_eq!(n.types, DEFAULT & !IMAGE);
        assert!(net("/Banner/*$match-case").body == "/Banner/");
        // Leading/trailing stars lift the anchors.
        let n = net("|*/ads/*|");
        assert!(!n.start && !n.end && n.body == "/ads/");
    }

    #[test]
    fn skips_what_it_does_not_understand() {
        for line in [
            "! comment",
            "[Adblock Plus 2.0]",
            "",
            "/ads[0-9]+/",
            "/banner/$script,domain=/x/",
            "||example.com^$redirect=noop.js",
            "||example.com^$csp=script-src 'none'",
            "@@||example.com^$elemhide",
            "||example.com^$unknownoption",
            "||exämple.com^",
            "*",     // everything
            "|http", // everything, almost
            "http",  // every address
            "://",
            ".js",
            "||com^", // a whole top-level domain
            "||co.uk^",
            "@@||com^$document",
            "$script,third-party",   // every third-party script
            "ad$domain=*.example.*", // only entity domains: would widen to all sites
            "example.com#?#div:-abp-has(.ad)",
            "example.com#$#abort-on-property-read x",
            "example.com##+js(nobab)",
            "##div:has-text(Sponsored)",
            "##.ad { background: url(https://tracker/) }",
            "##.ad}body{background:red",
            "##.ad /* comment",
            "##[title=\"unclosed]",
            "##.ad)",
            "##@import url(x)",
            "##html",
            "##body",
            "##*",
            "##:root",
            "example.com##html > body",
            "##.ad, body",
            "##body:not(.x)",
            "##html[lang]",
            "##html[lang=\"en\"]",
            "##:is(html)",
            "##:not(.ad)",
            "##[lang]",
            "##:not([data-x=\"1\"])",
            "##div, *",
            "##body > *",
            "||*.*^",
            "||a*.com^",
            "||*.co.uk^",
            ".com^",
            ".co.uk^",
            ".js?",
            "@@*$ping",
        ] {
            assert_eq!(parse(line), None, "{line}");
        }
        // Short patterns are fine when a site limits them, and specific ones without.
        assert!(parse("$script,domain=example.com").is_some());
        for ok in [
            "||t.co^",
            "||mkto-*.com^$third-party",
            "||bbc.co.uk^",
            "/ads/*",
            "-banner-ad.",
            "/ut.js?cb=",
            "|https://r.i.ua^",
            "||allsportsflix.",
            "*$ping,third-party",
            "##body > div.ad",
            "##.ad-slot",
            "example.com##html.ad",
        ] {
            assert!(parse(ok).is_some(), "{ok}");
        }
    }

    #[test]
    fn parses_hiding_rules() {
        let h = |l| match parse(l) {
            Some(Rule::Hide(h)) => h,
            o => panic!("{l}: {o:?}"),
        };
        assert_eq!(h("##.ad-slot").selector, ".ad-slot");
        let x = h("Example.com,~shop.example.com##div[id^=\"ad-\"]");
        assert_eq!(x.domains, vec!["example.com"]);
        assert_eq!(x.not_domains, vec!["shop.example.com"]);
        assert!(h("example.com#@#.ad").allow);
        // `#` inside an address is not a hiding rule.
        assert!(matches!(parse("||example.com/#ads"), Some(Rule::Net(_))));
    }

    fn webkit(list: &str) -> Vec<Value> {
        let parts = to_webkit(list.lines().filter_map(parse), 1000);
        parts
            .iter()
            .flat_map(|p| serde_json::from_str::<Vec<Value>>(p).unwrap())
            .collect()
    }

    #[test]
    fn converts_to_webkit_rules() {
        let rules = webkit(
            "||ads.example.com^\n/banner/*/img^$image,third-party\n|https://x.com/a.js|\n\
             @@||ads.example.com/ok.js$script\n@@||good.org^$document\n\
             ##.ad\n##.promo\nnews.com##.box\nnews.com#@#.promo\n~shop.com##.sponsor",
        );
        let filters: Vec<&str> = rules
            .iter()
            .map(|r| r["trigger"]["url-filter"].as_str().unwrap())
            .collect();
        // Generic hiding first, exceptions last.
        assert_eq!(rules[0]["action"]["type"], "css-display-none");
        assert_eq!(rules[0]["action"]["selector"], ".ad");
        assert_eq!(rules[0]["trigger"].get("unless-domain"), None);
        assert_eq!(
            rules.last().unwrap()["action"]["type"],
            "ignore-previous-rules"
        );
        assert!(filters.contains(&"^[^:]+://+([^/]*[.@])?ads\\.example\\.com[/:]"));
        assert!(filters.contains(&"/banner/.*/img([^a-zA-Z0-9_.%-].*)?$"));
        assert!(filters.contains(&"^https://x\\.com/a\\.js$"));
        let find = |f: &str| {
            rules
                .iter()
                .find(|r| r["trigger"]["url-filter"] == f)
                .unwrap()
        };
        let banner = find("/banner/.*/img([^a-zA-Z0-9_.%-].*)?$");
        assert_eq!(banner["trigger"]["load-type"], json!(["third-party"]));
        assert_eq!(
            banner["trigger"]["resource-type"],
            json!(["image", "svg-document"])
        );
        // A path rule without types leaves pages and frames alone; a host rule does not.
        let path = &find("^https://x\\.com/a\\.js$")["trigger"]["resource-type"];
        assert!(!path.as_array().unwrap().contains(&json!("document")));
        assert_eq!(
            find("^[^:]+://+([^/]*[.@])?ads\\.example\\.com[/:]")["trigger"].get("resource-type"),
            None
        );
        // Site exception, and a generic hide rule excepted on one site.
        let site = rules
            .iter()
            .find(|r| r["trigger"]["if-domain"] == json!(["*good.org"]));
        assert_eq!(site.unwrap()["action"]["type"], "ignore-previous-rules");
        let promo = rules
            .iter()
            .find(|r| r["action"]["selector"] == ".promo")
            .unwrap();
        assert_eq!(promo["trigger"]["unless-domain"], json!(["*news.com"]));
        let boxed = rules
            .iter()
            .find(|r| r["action"]["selector"] == ".box")
            .unwrap();
        assert_eq!(boxed["trigger"]["if-domain"], json!(["*news.com"]));
    }

    #[test]
    fn splits_into_parts_each_ending_with_exceptions() {
        let mut list: String = (0..2500)
            .map(|i| format!("||ad{i}.example.com^\n"))
            .collect();
        list.push_str("@@||ad1.example.com/ok^\n");
        let parts = to_webkit(list.lines().filter_map(parse), 1000);
        assert_eq!(parts.len(), 3);
        for p in &parts {
            let v: Vec<Value> = serde_json::from_str(p).unwrap();
            assert!(v.len() <= 1000);
            assert_eq!(v.last().unwrap()["action"]["type"], "ignore-previous-rules");
        }
    }

    #[test]
    fn glob_matching() {
        assert!(glob(b"ads^", b"ads/x", false, false));
        assert!(glob(b"ads^", b"ads", false, false));
        assert!(!glob(b"ads^", b"adsense", false, false));
        assert!(glob(b"/b*/img", b"http://x/b/1/2/img", false, true));
        assert!(!glob(b"/b*/img", b"http://x/b/1/2/im", false, true));
        assert!(glob(b"a.js", b"http://x/a.js", true, true));
        assert!(!glob(b"a.js", b"http://x/a.jsx", true, true));
        // Many stars stay quick.
        let s = vec![b'a'; 2000];
        assert!(!glob(b"*a*a*a*a*a*a*a*b", &s, false, true));
    }

    #[test]
    fn matcher_blocks_and_excepts() {
        let mut m = Matcher::default();
        let list = "||doubleclick.net^\n||ads.example.com^$third-party\n/banner/*$image\n\
                    @@||doubleclick.net/allowed^\n@@||good.org^$document\n\
                    ||tracker.io^$script,domain=news.com|~sport.news.com\nexample.org/x.js|";
        for rule in list.lines().filter_map(parse) {
            if let Rule::Net(n) = rule {
                m.add(n);
            }
        }
        let page = Some("https://www.news.com/story");
        assert!(m.blocks("https://stats.g.doubleclick.net/x", page, SCRIPT));
        assert!(!m.blocks("https://doubleclick.net/allowed/x", page, SCRIPT));
        assert!(!m.blocks("https://notdoubleclick.net/x", page, SCRIPT));
        assert!(m.blocks("https://ads.example.com/a", page, SCRIPT));
        assert!(!m.blocks(
            "https://ads.example.com/a",
            Some("https://example.com/"),
            SCRIPT
        ));
        assert!(m.blocks("https://cdn.x.com/banner/1.png", page, IMAGE));
        assert!(!m.blocks("https://cdn.x.com/banner/1.js", page, SCRIPT));
        assert!(!m.blocks("https://cdn.x.com/banner/1.png", page, DOC));
        assert!(m.blocks("https://tracker.io/t.js", page, SCRIPT));
        assert!(!m.blocks(
            "https://tracker.io/t.js",
            Some("https://sport.news.com/"),
            SCRIPT
        ));
        assert!(!m.blocks(
            "https://tracker.io/t.js",
            Some("https://other.com/"),
            SCRIPT
        ));
        assert!(m.blocks("https://example.org/x.js", page, SCRIPT));
        assert!(!m.blocks("https://example.org/x.jsx", page, SCRIPT));
        assert!(!m.blocks(
            "https://doubleclick.net/x",
            Some("https://good.org/"),
            SCRIPT
        ));
        assert!(!m.blocks("not a url", page, SCRIPT));
        assert_eq!(site_of("news.bbc.co.uk"), "bbc.co.uk");
        assert_eq!(site_of("a.b.example.com"), "example.com");
        assert_eq!(site_of("localhost"), "localhost");
    }

    #[test]
    fn garbage_never_panics() {
        let junk = "||\n@@\n|\n||^\n$\n@@$\n##\n#@#\n#\n$domain=\n||a^$domain=|||\n\
                    ||ab^$~\n\u{0}\u{1}\n||x.com^$third-party=,,\n@@||^$document\n\
                    ||é.com^\n##\u{7f}\n,##.a\n~##.a\n||a.com|$,|";
        let mut m = Matcher::default();
        let rules: Vec<Rule> = junk.lines().filter_map(parse).collect();
        let _ = to_webkit(junk.lines().filter_map(parse), 10);
        for r in rules {
            if let Rule::Net(n) = r {
                m.add(n);
            }
        }
        let _ = m.blocks("https://a.com/é?x=||", Some("ftp://"), SCRIPT);
    }
}

/// `WISP_LISTS=<folder of .txt lists> cargo test --release real_lists -- --ignored --nocapture`
#[cfg(test)]
#[test]
#[ignore]
fn real_lists() {
    let dir = std::env::var("WISP_LISTS").expect("WISP_LISTS");
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let text = std::fs::read_to_string(&path).unwrap();
        let t = std::time::Instant::now();
        let rules: Vec<Rule> = text.lines().filter_map(parse).collect();
        let parsed = t.elapsed();
        let (n, h) = rules.iter().fold((0, 0), |(n, h), r| match r {
            Rule::Net(_) => (n + 1, h),
            Rule::Hide(_) => (n, h + 1),
        });
        let lines = text
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('!'))
            .count();
        let parts = webkit::to_webkit(rules.into_iter(), 50_000);
        let wk: usize = parts.iter().map(|p| p.matches("\"trigger\"").count()).sum();
        let bytes: usize = parts.iter().map(String::len).sum();
        println!(
            "{}: {lines} lines -> {n} network + {h} hiding; {wk} WebKit rules in {} parts ({} KB); parse {parsed:?}, total {:?}",
            path.display(),
            parts.len(),
            bytes / 1024,
            t.elapsed()
        );
    }
}
