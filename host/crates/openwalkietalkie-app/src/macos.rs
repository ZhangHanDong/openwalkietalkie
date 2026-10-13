//! The two places the window reaches past gpui into AppKit, both about how
//! it looks and neither about what it does.
//!
//! **The blur.** gpui 0.2.2 blurs a `Blurred` window with an
//! `NSVisualEffectView` it inserts under its own view, set to the
//! `Selection` material. macOS 26 stopped producing a blur for that material,
//! which leaves the glass tint over an unblurred desktop. Zeron's gpui fork
//! moved to `UnderWindowBackground` for the same reason; this does the same
//! to the view gpui already made, so gpui's own hooks on it (the hidden
//! desktop-tint layer, the dropped saturation filter) still apply.
//!
//! **The pinned appearance.** [`crate::style::APPEARANCE_ENV`] sets
//! `NSApp.appearance`, so the traffic lights, the blur and the window's
//! reported appearance all agree with the palette instead of only the
//! palette changing.
use gpui::Window;
use objc2::msg_send;
use objc2::runtime::{AnyClass, AnyObject};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// `NSVisualEffectMaterialUnderWindowBackground`.
const UNDER_WINDOW_BACKGROUND: isize = 21;
/// `NSVisualEffectBlendingModeBehindWindow`.
const BEHIND_WINDOW: isize = 0;
/// `NSVisualEffectStateActive`: blurred whether or not the window has focus.
const ACTIVE: isize = 1;

/// The `NSWindow` gpui's view lives in.
fn ns_window(window: &Window) -> Option<*mut AnyObject> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return None;
    };
    let view = appkit.ns_view.as_ptr().cast::<AnyObject>();
    // SAFETY: `view` is gpui's live NSView; `window` is a getter, called on
    // the main thread where gpui runs.
    let ns_window: *mut AnyObject = unsafe { msg_send![view, window] };
    (!ns_window.is_null()).then_some(ns_window)
}

/// Re-material the effect view gpui put behind the window's content.
/// `false` when there is none, which is what a window opened without
/// `WindowBackgroundAppearance::Blurred` has, and what tells the caller to
/// paint the shell opaque.
pub fn use_under_window_material(window: &Window) -> bool {
    let Some(ns_window) = ns_window(window) else {
        return false;
    };
    let Some(effect_class) = AnyClass::get(c"NSVisualEffectView") else {
        return false;
    };
    // SAFETY: AppKit getters and the three documented NSVisualEffectView
    // setters, on views this window owns, on the main thread.
    unsafe {
        let content: *mut AnyObject = msg_send![ns_window, contentView];
        if content.is_null() {
            return false;
        }
        let subviews: *mut AnyObject = msg_send![content, subviews];
        if subviews.is_null() {
            return false;
        }
        let count: usize = msg_send![subviews, count];
        let mut found = false;
        for i in 0..count {
            let view: *mut AnyObject = msg_send![subviews, objectAtIndex: i];
            let is_effect: bool = msg_send![view, isKindOfClass: effect_class];
            if is_effect {
                let _: () = msg_send![view, setMaterial: UNDER_WINDOW_BACKGROUND];
                let _: () = msg_send![view, setBlendingMode: BEHIND_WINDOW];
                let _: () = msg_send![view, setState: ACTIVE];
                found = true;
            }
        }
        found
    }
}

/// Pin the whole app to Aqua (`dark == false`) or Dark Aqua.
pub fn pin_appearance(dark: bool) {
    let (Some(app_class), Some(appearance_class), Some(string_class)) = (
        AnyClass::get(c"NSApplication"),
        AnyClass::get(c"NSAppearance"),
        AnyClass::get(c"NSString"),
    ) else {
        return;
    };
    let name = if dark {
        c"NSAppearanceNameDarkAqua"
    } else {
        c"NSAppearanceNameAqua"
    };
    // SAFETY: class methods with documented signatures; the string is a
    // NUL-terminated literal and the app object is the shared instance.
    unsafe {
        let name: *mut AnyObject = msg_send![string_class, stringWithUTF8String: name.as_ptr()];
        if name.is_null() {
            return;
        }
        let appearance: *mut AnyObject = msg_send![appearance_class, appearanceNamed: name];
        if appearance.is_null() {
            return;
        }
        let app: *mut AnyObject = msg_send![app_class, sharedApplication];
        let _: () = msg_send![app, setAppearance: appearance];
    }
}
