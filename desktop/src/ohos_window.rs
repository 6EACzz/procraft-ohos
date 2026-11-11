//! The window HarmonyOS gives the app: an `OHNativeWindow` from the XComponent the ArkTS page
//! shows, and the wgpu surface on it. This is the tablet's `window.rs`, as Android's
//! (`crates/core/src/window.rs`) and the iPad's (`layer.rs`) are theirs: the only unsafe code of
//! the app's own, and the whole of what the platform's windowing needs.

#![allow(unsafe_code)]

use std::ffi::c_void;
use std::ptr::NonNull;

use procraft_gpu::wgpu;
use raw_window_handle::{OhosDisplayHandle, OhosNdkWindowHandle, RawDisplayHandle, RawWindowHandle};

/// An `OHNativeWindow` the XComponent handed over, held until the render thread lets go of it.
#[derive(Clone, Copy, Debug)]
pub struct NativeWindow(pub(crate) *mut c_void);

// SAFETY: an OHNativeWindow is a handle the system owns and any thread may draw into, as the
// Android app's ANativeWindow is (see `crates/core/src/window.rs`)
unsafe impl Send for NativeWindow {}
unsafe impl Sync for NativeWindow {}

#[link(name = "native_window")]
unsafe extern "C" {
    /// Counts one more holder of the window: the system frees it only when the count is nought.
    fn OH_NativeWindow_NativeObjectReference(obj: *mut c_void) -> i32;
    /// Lets go of one holder of the window.
    fn OH_NativeWindow_NativeObjectUnreference(obj: *mut c_void) -> i32;
}

/// Holds the window for as long as the app draws into it. The system destroys an XComponent's
/// window as soon as the shell has been told the surface went (as `OnSurfaceDestroyed` returns),
/// while the app's thread may still be drawing the frame it was in the middle of: holding the
/// window keeps it alive until [`let_go`] says the last frame is done, so what is drawn is drawn
/// into a window that is still there (nothing shows it, and nothing draws into freed memory).
pub fn hold(window: NativeWindow) -> NativeWindow {
    if !window.0.is_null() {
        // SAFETY: the handle is the live OHNativeWindow the shell was given
        unsafe { OH_NativeWindow_NativeObjectReference(window.0) };
    }
    window
}

/// Lets go of a window [`hold`] held, once nothing draws into it (the app's thread has stopped).
pub fn let_go(window: NativeWindow) {
    if !window.0.is_null() {
        // SAFETY: the caller holds the one reference `hold` took
        unsafe { OH_NativeWindow_NativeObjectUnreference(window.0) };
    }
}

/// A wgpu surface on `window`. The caller keeps `window` alive until the surface is dropped.
pub fn create_surface(instance: &wgpu::Instance, window: &NativeWindow) -> Result<wgpu::Surface<'static>, String> {
    let Some(handle) = NonNull::new(window.0) else { return Err("no window".to_owned()) };
    let target = wgpu::SurfaceTargetUnsafe::RawHandle {
        raw_display_handle: Some(RawDisplayHandle::Ohos(OhosDisplayHandle::new())),
        raw_window_handle: RawWindowHandle::OhosNdk(OhosNdkWindowHandle::new(handle)),
    };
    // SAFETY: the handle is a live OHNativeWindow (held by the caller as `hold` took it), and the
    // render thread, which owns both, drops the surface before the window
    unsafe { instance.create_surface_unsafe(target) }.map_err(|e| e.to_string())
}
