//! Ad blocking by host, done inside the engine's network layer on each platform:
//! WebKit content rules on macOS, a WebView2 request filter on Windows.
//!
//! ponytail: host list only (built-in + user-added), no cosmetic filtering and no
//! EasyList syntax. Upgrade path: the `adblock` crate plus downloadable filter lists.

use crate::config::Adblock;
use std::{collections::HashSet, sync::RwLock};

/// `None` = blocking off.
static HOSTS: RwLock<Option<HashSet<String>>> = RwLock::new(None);

fn valid_host(h: &str) -> bool {
    h.contains('.')
        && h.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

/// Load the block list from settings. Returns the hosts when blocking is on.
pub fn configure(cfg: &Adblock) -> Option<Vec<String>> {
    let hosts: Option<HashSet<String>> = cfg.enabled.then(|| {
        include_str!("blocklist.txt")
            .lines()
            .chain(cfg.extra_hosts.iter().map(String::as_str))
            .map(|l| l.trim().to_ascii_lowercase())
            .filter(|l| !l.starts_with('#') && valid_host(l))
            .collect()
    });
    let list = hosts.as_ref().map(|h| h.iter().cloned().collect());
    *HOSTS.write().unwrap() = hosts;
    list
}

fn host_of(url: &str) -> Option<&str> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    host.split(':').next().filter(|h| !h.is_empty())
}

/// True when the URL's host, or any parent domain of it, is on the block list.
pub fn is_blocked(url: &str) -> bool {
    let guard = HOSTS.read().unwrap();
    let (Some(set), Some(host)) = (guard.as_ref(), host_of(url)) else {
        return false;
    };
    let host = host.to_ascii_lowercase();
    let mut h = host.as_str();
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

// ---------------------------------------------------------------- macOS

#[cfg(target_os = "macos")]
mod platform {
    use block2::RcBlock;
    use objc2::{MainThreadMarker, rc::Retained};
    use objc2_foundation::{NSError, NSString};
    use objc2_web_kit::{WKContentRuleList, WKContentRuleListStore};
    use std::cell::RefCell;
    use wry::{WebView, WebViewExtMacOS};

    thread_local! {
        static RULES: RefCell<Option<Retained<WKContentRuleList>>> = const { RefCell::new(None) };
    }

    /// Compile the host list into WebKit content rules, then call `done` (on the main thread).
    pub fn compile(hosts: Option<Vec<String>>, done: impl Fn() + 'static) {
        RULES.set(None);
        let Some(hosts) = hosts else { return done() };
        let rules: Vec<_> = hosts
            .iter()
            .map(|h| {
                let filter = format!("^[^:]+://+([^:/]+\\.)?{}[:/]", h.replace('.', "\\."));
                serde_json::json!({"trigger": {"url-filter": filter}, "action": {"type": "block"}})
            })
            .collect();
        let json = serde_json::to_string(&rules).unwrap();
        let mtm = MainThreadMarker::new().expect("adblock must be set up on the main thread");
        let handler = RcBlock::new(move |list: *mut WKContentRuleList, err: *mut NSError| {
            // SAFETY: WebKit hands us either a valid rule list or null (with an error).
            match unsafe { Retained::retain(list) } {
                Some(list) => RULES.set(Some(list)),
                None => eprintln!("wisp: ad block rules failed to compile: {:?}", unsafe {
                    err.as_ref()
                }),
            }
            done();
        });
        // SAFETY: called on the main thread with non-null arguments.
        unsafe {
            let Some(store) = WKContentRuleListStore::defaultStore(mtm) else {
                return;
            };
            store.compileContentRuleListForIdentifier_encodedContentRuleList_completionHandler(
                Some(&NSString::from_str("wisp")),
                Some(&NSString::from_str(&json)),
                Some(&handler),
            );
        }
    }

    /// Make this webview use the current rules (or none, when blocking is off).
    pub fn attach(webview: &WebView) {
        let manager = webview.manager();
        // SAFETY: main-thread calls on a live WKUserContentController.
        unsafe {
            manager.removeAllContentRuleLists();
            RULES.with_borrow(|r| {
                if let Some(list) = r {
                    manager.addContentRuleList(list);
                }
            });
        }
    }

    pub const REATTACH_AFTER_COMPILE: bool = true;
}

// ---------------------------------------------------------------- Windows

#[cfg(windows)]
mod platform {
    use webview2_com::{
        Microsoft::Web::WebView2::Win32::COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
        WebResourceRequestedEventHandler, take_pwstr,
    };
    use windows_core::{HSTRING, PWSTR};
    use wry::{WebView, WebViewExtWindows};

    pub fn compile(_hosts: Option<Vec<String>>, done: impl Fn() + 'static) {
        done() // the request filter reads the live list; nothing to compile
    }

    /// Answer every request to a blocked host with an empty 403. Call once per webview.
    pub fn attach(webview: &WebView) {
        let core = webview.webview();
        let env = webview.environment();
        let handler = WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            // SAFETY: plain COM calls on objects WebView2 passed to this callback.
            unsafe {
                let mut uri = PWSTR::null();
                args.Request()?.Uri(&mut uri)?;
                if super::is_blocked(&take_pwstr(uri)) {
                    let response = env.CreateWebResourceResponse(
                        None,
                        403,
                        &HSTRING::from("Blocked"),
                        &HSTRING::new(),
                    )?;
                    args.SetResponse(&response)?;
                }
            }
            Ok(())
        }));
        // SAFETY: plain COM calls on a live webview.
        unsafe {
            let mut token = 0;
            let _ = core.AddWebResourceRequestedFilter(
                &HSTRING::from("*"),
                COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
            );
            let _ = core.add_WebResourceRequested(&handler, &mut token);
        }
    }

    pub const REATTACH_AFTER_COMPILE: bool = false;
}

// ---------------------------------------------------------------- other (Linux)

#[cfg(not(any(target_os = "macos", windows)))]
mod platform {
    // ponytail: no network-level blocking on Linux yet; only popups/new tabs are filtered.
    // Upgrade path: WebKitGTK UserContentFilterStore, same rule JSON as macOS.
    pub fn compile(_hosts: Option<Vec<String>>, done: impl Fn() + 'static) {
        done()
    }
    pub fn attach(_webview: &wry::WebView) {}
    pub const REATTACH_AFTER_COMPILE: bool = false;
}

pub use platform::{REATTACH_AFTER_COMPILE, attach, compile};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_listed_hosts_and_subdomains_only() {
        configure(&Adblock {
            enabled: true,
            extra_hosts: vec!["Ads.Example.org".into(), "bad host!".into()],
        });
        assert!(is_blocked("https://doubleclick.net/x"));
        assert!(is_blocked("https://stats.g.doubleclick.net:443/x?y"));
        assert!(is_blocked("http://user@ads.example.org/"));
        assert!(!is_blocked("https://example.org/"));
        assert!(!is_blocked("https://notdoubleclick.net/"));
        assert!(!is_blocked(
            "https://example.com/?u=https://doubleclick.net/"
        ));
        assert!(!is_blocked("about:blank"));

        configure(&Adblock {
            enabled: false,
            extra_hosts: vec![],
        });
        assert!(!is_blocked("https://doubleclick.net/x"));
    }
}
