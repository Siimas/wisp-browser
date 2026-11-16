//! The few things the webview library does not expose, per platform.

use tao::window::Window;
use wry::WebView;

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use objc2::{msg_send, runtime::AnyObject};
    use tao::platform::macos::WindowExtMacOS;
    use wry::WebViewExtMacOS;

    /// Clip a webview to rounded corners (the page "card").
    pub fn round_corners(webview: &WebView, radius: f64) {
        let view = webview.webview();
        // SAFETY: main-thread messages to a live NSView and its CALayer.
        unsafe {
            let _: () = msg_send![&*view, setWantsLayer: true];
            let layer: *mut AnyObject = msg_send![&*view, layer];
            if !layer.is_null() {
                let _: () = msg_send![layer, setCornerRadius: radius];
                let _: () = msg_send![layer, setMasksToBounds: true];
            }
        }
    }

    /// Put a webview above its siblings (re-adding a subview moves it to the front).
    pub fn raise(webview: &WebView, window: &Window) {
        let _ = webview.reparent(window.ns_window().cast());
    }

    /// The system web engine reports no browser name, and sites such as Google then
    /// serve a stripped-down legacy page. It is the same engine as the installed
    /// Safari, so identify as that Safari version.
    pub fn user_agent() -> Option<String> {
        let out = std::process::Command::new("/usr/bin/defaults")
            .args(["read", "/Applications/Safari.app/Contents/Info"])
            .arg("CFBundleShortVersionString")
            .output()
            .ok()?;
        let version = String::from_utf8(out.stdout).ok()?;
        let version = version.trim();
        let plausible =
            !version.is_empty() && version.chars().all(|c| c.is_ascii_digit() || c == '.');
        plausible.then(|| {
            format!(
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 \
                 (KHTML, like Gecko) Version/{version} Safari/605.1.15"
            )
        })
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::{
        HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos,
    };
    use wry::WebViewExtWindows;

    // ponytail: square page corners on Windows. Upgrade path: SetWindowRgn with a
    // rounded region, re-applied on every resize.
    pub fn round_corners(_webview: &WebView, _radius: f64) {}

    pub fn raise(webview: &WebView, _window: &Window) {
        let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
        // SAFETY: plain Win32 call on the webview's own live window.
        let _ = unsafe { SetWindowPos(webview.hwnd(), Some(HWND_TOP), 0, 0, 0, 0, flags) };
    }

    /// WebView2 already identifies as a full Chromium browser.
    pub fn user_agent() -> Option<String> {
        None
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod imp {
    use super::*;
    pub fn round_corners(_webview: &WebView, _radius: f64) {}
    pub fn raise(_webview: &WebView, _window: &Window) {}
    pub fn user_agent() -> Option<String> {
        None
    }
}

pub use imp::{raise, round_corners, user_agent};
