//! Development aid, debug builds on macOS only: save a picture of our own window.
//! `WISP_SHOT=/path/out.png` captures a few seconds after start and quits, so layout
//! changes can be checked without anyone at the screen. An app may capture its own
//! windows without the Screen Recording permission.

use objc2::{msg_send, rc::Retained, runtime::AnyObject};
use objc2_foundation::{NSString, NSURL};
use std::ffi::c_void;

#[repr(C)]
struct CGRect([f64; 4]);

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    static CGRectNull: CGRect;
    fn CGWindowListCreateImage(bounds: CGRect, list: u32, window: u32, image: u32) -> *mut c_void;
    fn CGImageRelease(image: *mut c_void);
}

#[link(name = "ImageIO", kind = "framework")]
unsafe extern "C" {
    fn CGImageDestinationCreateWithURL(
        url: *const c_void,
        kind: *const c_void,
        count: usize,
        options: *const c_void,
    ) -> *mut c_void;
    fn CGImageDestinationAddImage(dest: *mut c_void, image: *mut c_void, props: *const c_void);
    fn CGImageDestinationFinalize(dest: *mut c_void) -> bool;
}

/// `ns_window` is the window's `NSWindow*`.
pub fn save(ns_window: *mut c_void, path: &str) -> bool {
    const ONLY_THIS_WINDOW: u32 = 1 << 3;
    const WITHOUT_SHADOW: u32 = 1 << 0;
    let url = NSURL::fileURLWithPath(&NSString::from_str(path));
    let png = NSString::from_str("public.png");
    // SAFETY: plain C calls with valid arguments; NSURL/NSString are toll-free bridged
    // to the CFURL/CFString these functions expect.
    unsafe {
        let number: isize = msg_send![ns_window as *mut AnyObject, windowNumber];
        let rect = CGRect(CGRectNull.0);
        let image = CGWindowListCreateImage(rect, ONLY_THIS_WINDOW, number as u32, WITHOUT_SHADOW);
        if image.is_null() {
            return false;
        }
        let dest = CGImageDestinationCreateWithURL(
            Retained::as_ptr(&url).cast(),
            Retained::as_ptr(&png).cast(),
            1,
            std::ptr::null(),
        );
        let ok = !dest.is_null() && {
            CGImageDestinationAddImage(dest, image, std::ptr::null());
            CGImageDestinationFinalize(dest)
        };
        CGImageRelease(image);
        ok
    }
}
