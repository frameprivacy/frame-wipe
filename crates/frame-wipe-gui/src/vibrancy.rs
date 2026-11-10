//! Real window materials on macOS and Windows. Linux stays opaque and the
//! UI paints an emulated wallpaper instead.
//!
//! Uses crates already locked in this workspace (`objc` on macOS, `dwmapi`
//! on Windows) so the MSRV-sensitive lockfile does not need to change.

use raw_window_handle::HasWindowHandle;

/// Whether this build can ask the OS for a real translucent material.
pub fn supported() -> bool {
    cfg!(any(target_os = "macos", windows))
}

pub fn apply(handle: &impl HasWindowHandle, dark: bool) {
    let Ok(window) = handle.window_handle() else {
        return;
    };
    match window.as_raw() {
        #[cfg(target_os = "macos")]
        RawWindowHandle::AppKit(appkit) => unsafe { apply_macos(appkit.ns_view.as_ptr()) },
        #[cfg(windows)]
        RawWindowHandle::Win32(win) => unsafe {
            apply_windows(win.hwnd.get() as *mut std::ffi::c_void, dark)
        },
        _ => {
            let _ = dark;
        }
    }
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy)]
struct NsPoint {
    x: f64,
    y: f64,
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy)]
struct NsSize {
    width: f64,
    height: f64,
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy)]
struct NsRect {
    origin: NsPoint,
    size: NsSize,
}

#[cfg(target_os = "macos")]
unsafe fn apply_macos(ns_view: *mut std::ffi::c_void) {
    use objc::runtime::{Object, NO, YES};
    use objc::{class, msg_send, sel, sel_impl};

    if ns_view.is_null() {
        return;
    }
    let view: *mut Object = ns_view.cast();
    let window: *mut Object = msg_send![view, window];
    if window.is_null() {
        return;
    }
    let clear: *mut Object = msg_send![class!(NSColor), clearColor];
    let _: () = msg_send![window, setOpaque: NO];
    let _: () = msg_send![window, setBackgroundColor: clear];
    let _: () = msg_send![window, setTitlebarAppearsTransparent: YES];

    // NSVisualEffectMaterialUnderWindowBackground = 21
    // NSVisualEffectStateFollowsWindowActiveState = 0
    // NSVisualEffectBlendingModeBehindWindow = 0
    const UNDER_WINDOW: i64 = 21;
    const FOLLOWS_ACTIVE: i64 = 0;
    const BEHIND_WINDOW: i64 = 0;
    const AUTORESIZE_WH: u64 = 18;

    let bound: *mut Object = msg_send![window, contentView];
    if bound.is_null() {
        return;
    }
    let bounds: NsRect = msg_send![bound, bounds];
    let alloc: *mut Object = msg_send![class!(NSVisualEffectView), alloc];
    let effect: *mut Object = msg_send![alloc, initWithFrame: bounds];
    if effect.is_null() {
        return;
    }
    let _: () = msg_send![effect, setMaterial: UNDER_WINDOW];
    let _: () = msg_send![effect, setState: FOLLOWS_ACTIVE];
    let _: () = msg_send![effect, setBlendingMode: BEHIND_WINDOW];
    let _: () = msg_send![effect, setAutoresizingMask: AUTORESIZE_WH];
    let nil: *mut Object = std::ptr::null_mut();
    let _: () = msg_send![bound, addSubview: effect positioned: 0i64 relativeTo: nil];
}

#[cfg(windows)]
unsafe fn apply_windows(hwnd: *mut std::ffi::c_void, dark: bool) {
    if hwnd.is_null() {
        return;
    }
    // DWMWA_USE_IMMERSIVE_DARK_MODE = 20, DWMWA_SYSTEMBACKDROP_TYPE = 38
    // DWMSBT_MAINWINDOW (Mica) = 2
    #[link(name = "dwmapi")]
    extern "system" {
        fn DwmSetWindowAttribute(
            hwnd: *mut std::ffi::c_void,
            attr: u32,
            value: *const std::ffi::c_void,
            size: u32,
        ) -> i32;
    }
    let dark_i: i32 = if dark { 1 } else { 0 };
    let _ = DwmSetWindowAttribute(hwnd, 20, (&dark_i as *const i32).cast(), 4);
    let mica: i32 = 2;
    let _ = DwmSetWindowAttribute(hwnd, 38, (&mica as *const i32).cast(), 4);
}
