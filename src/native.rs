//! The few things the webview library does not expose, per platform.

use tao::window::Window;
use wry::WebView;

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use objc2::{
        ClassType, define_class, msg_send,
        rc::Retained,
        runtime::{AnyClass, AnyObject, NSObject, Sel},
        sel,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    use std::{
        cell::Cell,
        time::{Duration, Instant},
    };
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

    /// Does the page view hold the keyboard focus (is it, or something inside it, the
    /// window's first responder)?
    pub fn is_focused(webview: &WebView, window: &Window) -> bool {
        let view = webview.webview();
        // SAFETY: main-thread messages to the live window and views.
        unsafe {
            let responder: *mut AnyObject =
                msg_send![window.ns_window() as *mut AnyObject, firstResponder];
            let Some(class) = AnyClass::get(c"NSView").filter(|_| !responder.is_null()) else {
                return false;
            };
            let is_view: bool = msg_send![responder, isKindOfClass: class];
            is_view && msg_send![responder, isDescendantOf: &*view]
        }
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

    // ---- page tools: find in page, and the page's right-click menu ----

    /// Find `query` in the page, selecting and scrolling to the next (or previous)
    /// match. WebKit says only whether there was one, never how many.
    pub fn find(
        webview: &WebView,
        query: &str,
        back: bool,
        _fresh: bool,
        done: impl Fn(Found) + 'static,
    ) {
        let view = webview.webview();
        let reply = block2::RcBlock::new(move |result: *mut AnyObject| {
            // SAFETY: WebKit passes a live WKFindResult (or nothing).
            let found = !result.is_null() && unsafe { msg_send![result, matchFound] };
            done(Found { found, count: None });
        });
        // SAFETY: main-thread messages to live objects; WKFindConfiguration and
        // findString:withConfiguration:completionHandler: exist since macOS 11.
        unsafe {
            let Some(class) = AnyClass::get(c"WKFindConfiguration") else {
                return;
            };
            let config: *mut AnyObject = msg_send![class, new];
            let Some(config) = Retained::from_raw(config) else {
                return;
            };
            let _: () = msg_send![&*config, setBackwards: back];
            let query = objc2_foundation::NSString::from_str(query);
            let _: () = msg_send![&*view, findString: &*query, withConfiguration: &*config,
                completionHandler: &*reply];
        }
    }

    /// Nothing to end: WebKit's find leaves the last match selected, as Safari does.
    pub fn find_stop(_webview: &WebView) {}

    /// Is the page making sound, or using the camera or microphone? A muted or silent
    /// video does not count. An older system that cannot say counts as busy, so
    /// nothing that might be playing is unloaded.
    /// (WebKit's public `requestMediaPlaybackState` does not answer this: it reports
    /// whether the app paused the page's media, and says "playing" for any page with
    /// a video on it. `_isPlayingAudio` is what Safari's tab speaker icon follows.)
    pub fn media_busy(webview: &WebView, done: impl Fn(bool)) {
        let view = webview.webview();
        // SAFETY: main-thread messages to a live WKWebView, each checked to exist
        // first (the capture states since macOS 12).
        let busy = unsafe {
            let has = |sel| -> bool { msg_send![&*view, respondsToSelector: sel] };
            if !has(sel!(_isPlayingAudio)) || !has(sel!(cameraCaptureState)) {
                return done(true);
            }
            let audio: bool = msg_send![&*view, _isPlayingAudio];
            // WKMediaCaptureState: 0 none, 1 active, 2 muted (still in the call).
            let camera: isize = msg_send![&*view, cameraCaptureState];
            let microphone: isize = msg_send![&*view, microphoneCaptureState];
            audio || camera != 0 || microphone != 0
        };
        done(busy)
    }

    /// WebKit lowers a hidden page's priority by itself.
    pub fn set_background(_webview: &WebView, _background: bool) {}

    thread_local! {
        /// The page view whose right-click menu was opened last.
        static MENU_VIEW: Cell<usize> = const { Cell::new(0) };
        /// "Open Link in Private Tab" was chosen in the menu of this page view, then.
        static PRIVATE_PICK: Cell<Option<(usize, Instant)>> = const { Cell::new(None) };
        static MENU_TARGET: Retained<MenuTarget> = unsafe { msg_send![MenuTarget::class(), new] };
    }

    /// How long a choice of "Open Link in Private Tab" waits for its link. WebKit does
    /// not tell us which link the menu was for; it opens the link as a new-window
    /// request a moment later, and the first such request from that page is taken to
    /// be it (see `take_private`).
    const PRIVATE_PICK_LASTS: Duration = Duration::from_secs(1);

    define_class!(
        // SAFETY: NSObject has no subclassing requirements, and this type has no Drop.
        #[unsafe(super(NSObject))]
        #[name = "WispMenuTarget"]
        struct MenuTarget;

        impl MenuTarget {
            /// Our "Open Link in Private Tab" item was clicked. It carries WebKit's
            /// own "open link in new window" item: note the wish, then press that.
            #[unsafe(method(openLinkPrivate:))]
            fn open_link_private(&self, sender: &AnyObject) {
                PRIVATE_PICK.set(Some((MENU_VIEW.get(), Instant::now())));
                // SAFETY: main-thread messages to live AppKit objects.
                unsafe {
                    let original: *mut AnyObject = msg_send![sender, representedObject];
                    if original.is_null() {
                        return;
                    }
                    let action: Option<Sel> = msg_send![original, action];
                    let target: *mut AnyObject = msg_send![original, target];
                    let (Some(action), Some(app)) = (action, AnyClass::get(c"NSApplication"))
                    else {
                        return;
                    };
                    let app: *mut AnyObject = msg_send![app, sharedApplication];
                    let _: bool = msg_send![app, sendAction: action, to: target, from: original];
                }
            }
        }
    );

    /// Runs as the page view's `willOpenMenu:withEvent:`, just before WebKit shows its
    /// right-click menu. WebKit's "open in new window" items already arrive in the app
    /// as new-window requests, which open tabs: say so, and offer a private tab too.
    unsafe extern "C-unwind" fn will_open_menu(
        this: &AnyObject,
        _cmd: Sel,
        menu: *mut AnyObject,
        event: *mut AnyObject,
    ) {
        let title = |s: &str| objc2_foundation::NSString::from_str(s);
        // SAFETY: main-thread messages to the live view, its menu and the menu's items.
        unsafe {
            if let Some(webkit) = AnyClass::get(c"WKWebView") {
                let _: () = msg_send![super(this, webkit), willOpenMenu: menu, withEvent: event];
            }
            MENU_VIEW.set(this as *const AnyObject as usize);
            let mut link_item = None;
            let count: isize = msg_send![menu, numberOfItems];
            for i in 0..count {
                let item: *mut AnyObject = msg_send![menu, itemAtIndex: i];
                let id: *mut objc2_foundation::NSString = msg_send![item, identifier];
                let new_title = match id.as_ref().map(|id| id.to_string()).as_deref() {
                    Some("WKMenuItemIdentifierOpenLinkInNewWindow") => {
                        link_item = Some((i, item));
                        "Open Link in New Tab"
                    }
                    Some("WKMenuItemIdentifierOpenImageInNewWindow") => "Open Image in New Tab",
                    Some("WKMenuItemIdentifierOpenMediaInNewWindow") => "Open Video in New Tab",
                    Some("WKMenuItemIdentifierCopyLink") => "Copy Link Address",
                    _ => continue,
                };
                let _: () = msg_send![item, setTitle: &*title(new_title)];
            }
            let (Some((at, original)), Some(class)) = (link_item, AnyClass::get(c"NSMenuItem"))
            else {
                return;
            };
            let item: *mut AnyObject = msg_send![class, alloc];
            let item: *mut AnyObject = msg_send![item, initWithTitle: &*title("Open Link in Private Tab"),
                action: sel!(openLinkPrivate:), keyEquivalent: &*title("")];
            let Some(item) = Retained::from_raw(item) else {
                return;
            };
            MENU_TARGET.with(|target| {
                let _: () = msg_send![&*item, setTarget: &**target];
            });
            let _: () = msg_send![&*item, setRepresentedObject: original];
            let _: () = msg_send![menu, insertItem: &*item, atIndex: at + 1];
        }
    }

    /// Adjust the right-click menu of pages. `_pick` is unused here: the menu's items
    /// reach the app as ordinary new-window requests (see `take_private`).
    pub fn page_menu(webview: &WebView, _pick: impl Fn(String, bool) + 'static) {
        static ONCE: std::sync::Once = std::sync::Once::new();
        let view = webview.webview();
        ONCE.call_once(|| {
            let class: *const AnyClass = (*view).class();
            // SAFETY: adds a method the webview class does not define itself (it then
            // overrides NSView's), with the matching signature: void, self, _cmd, two objects.
            unsafe {
                let imp: objc2::runtime::Imp = std::mem::transmute(
                    will_open_menu
                        as unsafe extern "C-unwind" fn(
                            &AnyObject,
                            Sel,
                            *mut AnyObject,
                            *mut AnyObject,
                        ),
                );
                let sel = sel!(willOpenMenu:withEvent:);
                objc2::ffi::class_addMethod(class.cast_mut(), sel, imp, c"v@:@@".as_ptr());
            }
        });
    }

    /// A page asked for a new window: is this the link the user just chose "Open Link
    /// in Private Tab" for? The choice is used up by the first request from that page,
    /// so it never makes a second, unrelated tab private.
    /// ponytail: what remains is a race. A page that opens a popup by itself in the
    /// instant between the click and WebKit delivering the link takes the choice: the
    /// popup opens private and the link as a normal tab. The page cannot see the click,
    /// so it would have to be opening popups constantly. Upgrade path: WebKit's
    /// (private) context-menu element info, which names the link.
    pub fn take_private(webview: &WebView) -> bool {
        let view = Retained::as_ptr(&webview.webview()) as usize;
        match PRIVATE_PICK.get() {
            Some((v, at)) if v == view => {
                PRIVATE_PICK.set(None);
                at.elapsed() < PRIVATE_PICK_LASTS
            }
            _ => false,
        }
    }

    // ---- low power: what the system says, read when asked (each read is cheap) ----

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOPSGetBatteryWarningLevel() -> i32;
        fn IOPSNotificationCreateRunLoopSource(
            callback: extern "C" fn(*mut std::ffi::c_void),
            context: *mut std::ffi::c_void,
        ) -> *mut std::ffi::c_void;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFRunLoopCommonModes: *const std::ffi::c_void;
        fn CFRunLoopGetMain() -> *mut std::ffi::c_void;
        fn CFRunLoopAddSource(
            rl: *mut std::ffi::c_void,
            source: *mut std::ffi::c_void,
            mode: *const std::ffi::c_void,
        );
    }
    unsafe extern "C" {
        fn sysctlbyname(
            name: *const std::ffi::c_char,
            old: *mut std::ffi::c_void,
            len: *mut usize,
            new: *const std::ffi::c_void,
            newlen: usize,
        ) -> i32;
    }

    pub fn power() -> Power {
        // SAFETY: plain reads of system state, on the main thread for the AppKit ones;
        // isLowPowerModeEnabled exists since macOS 12, checked before it is sent.
        unsafe {
            let info: *mut AnyObject =
                msg_send![AnyClass::get(c"NSProcessInfo").unwrap(), processInfo];
            let saver = msg_send![info, respondsToSelector: sel!(isLowPowerModeEnabled)]
                && msg_send![info, isLowPowerModeEnabled];
            let ws: *mut AnyObject =
                msg_send![AnyClass::get(c"NSWorkspace").unwrap(), sharedWorkspace];
            let reduce_transparency = msg_send![ws, accessibilityDisplayShouldReduceTransparency];
            // Early (2) or final (3) warning: on battery and below about 20%.
            let low_battery = IOPSGetBatteryWarningLevel() >= 2;
            let (mut memory, mut len) = (0u64, size_of::<u64>());
            sysctlbyname(
                c"hw.memsize".as_ptr(),
                (&raw mut memory).cast(),
                &mut len,
                std::ptr::null(),
                0,
            );
            Power {
                saver,
                low_battery,
                memory,
                reduce_transparency,
            }
        }
    }

    thread_local! {
        static ON_POWER: std::cell::RefCell<Option<Box<dyn Fn()>>> = const { std::cell::RefCell::new(None) };
    }

    extern "C" fn power_changed(_: *mut std::ffi::c_void) {
        ON_POWER.with_borrow(|f| f.as_ref().map(|f| f()));
    }

    /// Call `f` when the power source or battery level changes (the system tells us; no
    /// polling). The system's low-power and accessibility switches send nothing here:
    /// they are read again when the window regains focus.
    pub fn on_power_change(f: impl Fn() + 'static) {
        ON_POWER.set(Some(Box::new(f)));
        // SAFETY: a run-loop source on the main run loop, which this (main) thread runs;
        // it lives for the rest of the process.
        unsafe {
            let source = IOPSNotificationCreateRunLoopSource(power_changed, std::ptr::null_mut());
            if !source.is_null() {
                CFRunLoopAddSource(CFRunLoopGetMain(), source, kCFRunLoopCommonModes);
            }
        }
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::rc::Rc;
    use webview2_com::{
        ContextMenuRequestedEventHandler, CustomItemSelectedEventHandler,
        FindStartCompletedHandler,
        Microsoft::Web::WebView2::Win32::{
            COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND_COMMAND,
            COREWEBVIEW2_CONTEXT_MENU_TARGET_KIND_IMAGE,
            COREWEBVIEW2_CONTEXT_MENU_TARGET_KIND_PAGE, ICoreWebView2_8, ICoreWebView2_11,
            ICoreWebView2_28, ICoreWebView2Environment9, ICoreWebView2Environment15,
        },
        take_pwstr,
    };
    use windows::Win32::System::Com::IStream;
    use windows::Win32::UI::WindowsAndMessaging::{
        GUITHREADINFO, GetGUIThreadInfo, HWND_TOP, IsChild, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        SetWindowPos,
    };
    use windows_core::{BOOL, HSTRING, Interface, PWSTR};
    use wry::WebViewExtWindows;

    // ---- the maximize button, as the system sees it ----

    use std::cell::{Cell, RefCell};
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows::Win32::Graphics::Gdi::ScreenToClient;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, GetParent, HTMAXBUTTON, HTTOP, IsZoomed, PostMessageW,
        RegisterClassW, WINDOW_EX_STYLE, WM_NCHITTEST, WM_NCLBUTTONDOWN, WM_NCLBUTTONUP,
        WM_NCMOUSELEAVE, WM_NCMOUSEMOVE, WNDCLASSW, WS_CHILD, WS_CLIPSIBLINGS, WS_VISIBLE,
    };

    type SnapSend = Option<Box<dyn Fn(&'static str)>>;
    thread_local! {
        static SNAP_SEND: RefCell<SnapSend> = const { RefCell::new(None) };
        static SNAP_HOVER: Cell<bool> = const { Cell::new(false) };
        static SNAP_PRESSED: Cell<bool> = const { Cell::new(false) };
        /// It lies along the window's top edge (not on a floating bar further down).
        static SNAP_AT_TOP: Cell<bool> = const { Cell::new(false) };
    }

    /// Windows 11 offers its snap layouts while the pointer rests on a window's maximize
    /// button, which it finds by asking the window what lies under the pointer. The
    /// interface draws that button inside a web view the system cannot ask, so this
    /// small window, drawing nothing, lies over it and answers "the maximize button".
    /// What happens on it (hover, press, leave, click) goes to `on`, for the interface
    /// to show and the app to act on.
    pub struct SnapButton(HWND);

    impl SnapButton {
        pub fn new(window: &Window, on: impl Fn(&'static str) + 'static) -> Option<Self> {
            use tao::platform::windows::WindowExtWindows;
            SNAP_SEND.set(Some(Box::new(on)));
            let class = windows_core::w!("WispSnapButton");
            // SAFETY: registering a class and creating a child of our own live window.
            unsafe {
                let instance = GetModuleHandleW(None).ok()?.into();
                RegisterClassW(&WNDCLASSW {
                    lpfnWndProc: Some(snap_proc),
                    hInstance: instance,
                    lpszClassName: class,
                    ..Default::default()
                });
                let style = WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS;
                let parent = HWND(window.hwnd() as _);
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    class,
                    None,
                    style,
                    0,
                    0,
                    0,
                    0,
                    Some(parent),
                    None,
                    Some(instance),
                    None,
                )
                .ok()
                .map(Self)
            }
        }

        /// Lie over `rect` (left, top, width, height in logical pixels), above the web views.
        pub fn place(&self, [l, t, w, h]: [f64; 4], scale: f64) {
            let p = |v: f64| (v * scale).round() as i32;
            let (x, y) = (p(l), p(t));
            SNAP_AT_TOP.set(y == 0);
            // SAFETY: plain Win32 call on our own live window.
            let _ = unsafe {
                SetWindowPos(
                    self.0,
                    Some(HWND_TOP),
                    x,
                    y,
                    p(l + w) - x,
                    p(t + h) - y,
                    SWP_NOACTIVATE,
                )
            };
        }
    }

    unsafe extern "system" fn snap_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        let send = |what| SNAP_SEND.with_borrow(|f| f.as_ref().map(|f| f(what)));
        // SAFETY (whole body): Win32 calls on this window and its parent, from its own thread.
        unsafe {
            match msg {
                WM_NCHITTEST => {
                    // Its top few pixels are still the window's top edge, as everywhere else
                    // along the top (see #rim in ui.html), unless maximized or it is not
                    // at the top (on a revealed top bar).
                    let mut pt = POINT {
                        x: (l.0 & 0xffff) as i16 as i32,
                        y: ((l.0 >> 16) & 0xffff) as i16 as i32,
                    };
                    let _ = ScreenToClient(hwnd, &mut pt);
                    let parent = GetParent(hwnd).unwrap_or_default();
                    let rim = (5 * windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd)) as i32 / 96;
                    let top = SNAP_AT_TOP.get() && pt.y < rim && !IsZoomed(parent).as_bool();
                    LRESULT((if top { HTTOP } else { HTMAXBUTTON }) as isize)
                }
                WM_NCLBUTTONDOWN if w.0 == HTTOP as usize => {
                    let parent = GetParent(hwnd).unwrap_or_default();
                    let _ = PostMessageW(Some(parent), msg, w, l); // resize the window, not this
                    LRESULT(0)
                }
                WM_NCMOUSEMOVE => {
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE | TME_NONCLIENT,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    let _ = TrackMouseEvent(&mut track);
                    if w.0 == HTMAXBUTTON as usize && !SNAP_HOVER.replace(true) {
                        send("hover");
                    } else if w.0 != HTMAXBUTTON as usize && SNAP_HOVER.replace(false) {
                        send("leave");
                    }
                    LRESULT(0)
                }
                WM_NCMOUSELEAVE => {
                    SNAP_HOVER.set(false);
                    SNAP_PRESSED.set(false);
                    send("leave");
                    LRESULT(0)
                }
                WM_NCLBUTTONDOWN => {
                    SNAP_PRESSED.set(true);
                    send("press");
                    LRESULT(0)
                }
                WM_NCLBUTTONUP if SNAP_PRESSED.replace(false) => {
                    send("click");
                    LRESULT(0)
                }
                _ => DefWindowProcW(hwnd, msg, w, l),
            }
        }
    }

    // ponytail: square page corners on Windows. Upgrade path: SetWindowRgn with a
    // rounded region, re-applied on every resize.
    pub fn round_corners(_webview: &WebView, _radius: f64) {}

    pub fn raise(webview: &WebView, _window: &Window) {
        let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
        // SAFETY: plain Win32 call on the webview's own live window.
        let _ = unsafe { SetWindowPos(webview.hwnd(), Some(HWND_TOP), 0, 0, 0, 0, flags) };
    }

    /// Does the page hold the keyboard focus? The focused window of the foreground
    /// thread (WebView2 draws pages in windows of its own process) lies inside its view.
    pub fn is_focused(webview: &WebView, _window: &Window) -> bool {
        let mut info = GUITHREADINFO {
            cbSize: size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        let host = webview.hwnd();
        // SAFETY: plain Win32 calls with a properly sized struct.
        unsafe {
            GetGUIThreadInfo(0, &mut info).is_ok()
                && (info.hwndFocus == host || IsChild(host, info.hwndFocus).as_bool())
        }
    }

    /// WebView2 already identifies as a full Chromium browser.
    pub fn user_agent() -> Option<String> {
        None
    }

    // ---- page tools: find in page, and the page's right-click menu ----

    /// Find `query` with WebView2's own find (it highlights every match and counts
    /// them), without its dialog. An older WebView2 without it finds nothing: searching
    /// by page script instead would let the page see what is being searched for.
    pub fn find(
        webview: &WebView,
        query: &str,
        back: bool,
        fresh: bool,
        done: impl Fn(Found) + 'static,
    ) {
        let native = || -> windows_core::Result<()> {
            // SAFETY: COM calls on the live webview, on its own thread.
            unsafe {
                let find = webview.webview().cast::<ICoreWebView2_28>()?.Find()?;
                if !fresh {
                    return if back {
                        find.FindPrevious()
                    } else {
                        find.FindNext()
                    };
                }
                let environment = webview.environment().cast::<ICoreWebView2Environment15>()?;
                let options = environment.CreateFindOptions()?;
                options.SetFindTerm(&HSTRING::from(query))?;
                options.SetSuppressDefaultFindDialog(true)?;
                options.SetShouldHighlightAllMatches(true)?;
                let session = find.clone();
                let started = FindStartCompletedHandler::create(Box::new(move |result| {
                    result?;
                    let mut count = 0;
                    session.MatchCount(&mut count)?;
                    done(Found {
                        found: count > 0,
                        count: Some(count.max(0) as u32),
                    });
                    Ok(())
                }));
                find.Start(&options, &started)
            }
        };
        let _ = native();
    }

    pub fn find_stop(webview: &WebView) {
        // SAFETY: as above.
        unsafe {
            if let Ok(find) = webview
                .webview()
                .cast::<ICoreWebView2_28>()
                .and_then(|w| w.Find())
            {
                let _ = find.Stop();
            }
        }
    }

    /// Is the page making sound? WebView2 does not say whether the camera or
    /// microphone is in use, so a call where everyone is silent counts as idle.
    /// Untested: written without a Windows machine at hand.
    pub fn media_busy(webview: &WebView, done: impl Fn(bool)) {
        let mut playing = BOOL(1);
        // SAFETY: COM call on the live webview, on its own thread.
        let asked = unsafe {
            webview
                .webview()
                .cast::<ICoreWebView2_8>()
                .and_then(|w| w.IsDocumentPlayingAudio(&mut playing))
        };
        done(asked.is_err() || playing.as_bool()); // unknown counts as playing
    }

    /// A hidden page asks WebView2 to keep less in memory (it may page some out); a
    /// cheap first step before unloading. Untested, like `media_busy`.
    pub fn set_background(webview: &WebView, background: bool) {
        use wry::MemoryUsageLevel::{Low, Normal};
        let _ = webview.set_memory_usage_level(if background { Low } else { Normal });
    }

    /// Add link and image entries to the top of WebView2's right-click menu. A chosen
    /// entry calls `pick(address, private)`: the address comes from WebView2 itself
    /// (what was under the cursor), and the call from the user's click on a native
    /// menu, so the page has no say in either.
    pub fn page_menu(webview: &WebView, pick: impl Fn(String, bool) + 'static) {
        let environment = webview.environment();
        let pick = Rc::new(pick);
        let handler = ContextMenuRequestedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            // SAFETY: COM calls on the arguments WebView2 just handed us.
            unsafe {
                let target = args.ContextMenuTarget()?;
                let items = args.MenuItems()?;
                let environment = environment.cast::<ICoreWebView2Environment9>()?;
                let mut at = 0;
                let mut add = |label: &str, url: &str, private: bool| -> windows_core::Result<()> {
                    let item = environment.CreateContextMenuItem(
                        &HSTRING::from(label),
                        None::<&IStream>,
                        COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND_COMMAND,
                    )?;
                    let (pick, url) = (pick.clone(), url.to_string());
                    let chosen = CustomItemSelectedEventHandler::create(Box::new(move |_, _| {
                        pick(url.clone(), private);
                        Ok(())
                    }));
                    item.add_CustomItemSelected(&chosen, &mut 0)?;
                    items.InsertValueAtIndex(at, &item)?;
                    at += 1;
                    Ok(())
                };
                let mut has = BOOL::default();
                target.HasLinkUri(&mut has)?;
                if has.as_bool() {
                    let mut link = PWSTR::null();
                    target.LinkUri(&mut link)?;
                    let link = take_pwstr(link);
                    add("Open link in new tab", &link, false)?;
                    add("Open link in private tab", &link, true)?;
                }
                let mut kind = COREWEBVIEW2_CONTEXT_MENU_TARGET_KIND_PAGE;
                target.Kind(&mut kind)?;
                target.HasSourceUri(&mut has)?;
                if kind == COREWEBVIEW2_CONTEXT_MENU_TARGET_KIND_IMAGE && has.as_bool() {
                    let mut image = PWSTR::null();
                    target.SourceUri(&mut image)?;
                    add("Open image in new tab", &take_pwstr(image), false)?;
                }
            }
            Ok(())
        }));
        // SAFETY: COM call on the live webview, on its own thread.
        unsafe {
            if let Ok(webview) = webview.webview().cast::<ICoreWebView2_11>() {
                let _ = webview.add_ContextMenuRequested(&handler, &mut 0);
            }
        }
    }

    /// Private tabs are asked for through `page_menu`'s callback here.
    pub fn take_private(_webview: &WebView) -> bool {
        false
    }

    pub fn power() -> Power {
        use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
        use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut st = SYSTEM_POWER_STATUS::default();
        let mut mem = MEMORYSTATUSEX {
            dwLength: size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        // SAFETY: plain Win32 reads into structs we own.
        let (have_st, have_mem) = unsafe {
            (
                GetSystemPowerStatus(&mut st).is_ok(),
                GlobalMemoryStatusEx(&mut mem).is_ok(),
            )
        };
        Power {
            saver: have_st && st.SystemStatusFlag == 1,
            // On battery (0) at 20% or less (255 = unknown).
            low_battery: have_st && st.ACLineStatus == 0 && st.BatteryLifePercent <= 20,
            memory: if have_mem { mem.ullTotalPhys } else { 0 },
            reduce_transparency: false, // no glass on Windows to reduce
        }
    }

    // ponytail: Windows re-reads on focus only. Upgrade path: WM_POWERBROADCAST via a
    // window subclass, if a plugged-in laptop should switch while Wisp is in front.
    pub fn on_power_change(_f: impl Fn() + 'static) {}
}

#[cfg(not(any(target_os = "macos", windows)))]
mod imp {
    use super::*;
    pub fn round_corners(_webview: &WebView, _radius: f64) {}
    pub fn raise(_webview: &WebView, _window: &Window) {}
    pub fn is_focused(_webview: &WebView, _window: &Window) -> bool {
        false
    }
    pub fn user_agent() -> Option<String> {
        None
    }
    pub fn find(
        _webview: &WebView,
        _query: &str,
        _back: bool,
        _fresh: bool,
        _done: impl Fn(Found),
    ) {
    }
    pub fn find_stop(_webview: &WebView) {}
    pub fn media_busy(_webview: &WebView, done: impl Fn(bool)) {
        done(true)
    }
    pub fn set_background(_webview: &WebView, _background: bool) {}
    pub fn page_menu(_webview: &WebView, _pick: impl Fn(String, bool)) {}
    pub fn take_private(_webview: &WebView) -> bool {
        false
    }
    pub fn power() -> Power {
        Power::default()
    }
    pub fn on_power_change(_f: impl Fn() + 'static) {}
}

/// What a search of the page came to. `count` only where the platform counts matches.
pub struct Found {
    pub found: bool,
    pub count: Option<u32>,
}

/// What the system says about power and display preferences, for low-power mode.
#[derive(Default, Clone, Copy, PartialEq, Debug)]
pub struct Power {
    /// The system's own low-power mode (macOS) or battery saver (Windows) is on.
    pub saver: bool,
    /// On battery and running low (about 20% or less).
    pub low_battery: bool,
    /// Physical memory in bytes, 0 if unknown.
    pub memory: u64,
    /// The system asks for less transparency (macOS accessibility).
    pub reduce_transparency: bool,
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
#[cfg(windows)]
pub use imp::SnapButton;
pub use imp::{
    find, find_stop, is_focused, media_busy, on_power_change, page_menu, power, raise,
    round_corners, set_background, take_private, user_agent,
};
