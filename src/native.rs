//! The few things the webview library does not expose, per platform.

use tao::window::Window;
use wry::WebView;

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use objc2::{
        msg_send,
        rc::Retained,
        runtime::{AnyClass, AnyObject},
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize};
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

    /// A pane of frosted glass: it blurs whatever is behind it in the window. Placed
    /// directly under the floating bar, whose own background is see-through.
    pub struct Glass(Retained<AnyObject>);

    impl Glass {
        pub fn new() -> Option<Self> {
            const POPOVER: isize = 6; // material: follows light/dark
            const WITHIN_WINDOW: isize = 1; // blur our own window's content, not the desktop
            const ACTIVE: isize = 1; // stay frosted even when the window is not key
            // SAFETY: standard AppKit object creation on the main thread.
            unsafe {
                let class = AnyClass::get(c"NSVisualEffectView")?;
                let view: *mut AnyObject = msg_send![class, alloc];
                let view: *mut AnyObject = msg_send![view, init];
                let view = Retained::from_raw(view)?;
                let _: () = msg_send![&*view, setMaterial: POPOVER];
                let _: () = msg_send![&*view, setBlendingMode: WITHIN_WINDOW];
                let _: () = msg_send![&*view, setState: ACTIVE];
                let _: () = msg_send![&*view, setWantsLayer: true];
                let _: () = msg_send![&*view, setHidden: true];
                Some(Self(view))
            }
        }

        /// Bring the pane to the front of the window's views. Raise the floating bar
        /// right after, so the pane ends up directly beneath it.
        pub fn restack(&self, window: &Window) {
            // SAFETY: main-thread messages to live views.
            unsafe {
                let content: *mut AnyObject =
                    msg_send![window.ns_window() as *mut AnyObject, contentView];
                if !content.is_null() {
                    let _: () = msg_send![content, addSubview: &*self.0];
                }
            }
        }

        /// Show the pane at `[x, y, w, h]` (window coordinates, top-left origin), fading in.
        pub fn show(&self, window: &Window, [x, y, w, h]: [f64; 4], radius: f64) {
            self.restack(window);
            // SAFETY: main-thread messages to live views.
            unsafe {
                let content: *mut AnyObject =
                    msg_send![window.ns_window() as *mut AnyObject, contentView];
                if content.is_null() {
                    return;
                }
                let bounds: NSRect = msg_send![content, bounds];
                let flipped: bool = msg_send![content, isFlipped];
                let y = if flipped {
                    y
                } else {
                    bounds.size.height - y - h
                };
                let frame = NSRect::new(NSPoint::new(x, y), NSSize::new(w.max(1.0), h.max(1.0)));
                let _: () = msg_send![&*self.0, setFrame: frame];
                let layer: *mut AnyObject = msg_send![&*self.0, layer];
                if !layer.is_null() {
                    let _: () = msg_send![layer, setCornerRadius: radius];
                    let _: () = msg_send![layer, setMasksToBounds: true];
                }
                let was_hidden: bool = msg_send![&*self.0, isHidden];
                if was_hidden {
                    let _: () = msg_send![&*self.0, setAlphaValue: 0.0f64];
                    let _: () = msg_send![&*self.0, setHidden: false];
                    let animator: *mut AnyObject = msg_send![&*self.0, animator];
                    let _: () = msg_send![animator, setAlphaValue: 1.0f64];
                }
            }
        }

        pub fn hide(&self) {
            // SAFETY: main-thread message to a live view.
            unsafe {
                let _: () = msg_send![&*self.0, setHidden: true];
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

/// No frosted glass off macOS yet: the floating bar stays opaque there.
#[cfg(not(target_os = "macos"))]
pub struct Glass;

#[cfg(not(target_os = "macos"))]
impl Glass {
    pub fn new() -> Option<Self> {
        None
    }
    pub fn restack(&self, _window: &Window) {}
    pub fn show(&self, _window: &Window, _frame: [f64; 4], _radius: f64) {}
    pub fn hide(&self) {}
}

#[cfg(target_os = "macos")]
pub use imp::Glass;
pub use imp::{raise, round_corners, user_agent};
