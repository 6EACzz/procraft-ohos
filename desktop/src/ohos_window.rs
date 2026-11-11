//! The window HarmonyOS gives the app: an `OHNativeWindow` from the XComponent the ArkTS page
//! shows, and the wgpu surface on it. This is the tablet's `window.rs`, as Android's
//! (`crates/core/src/window.rs`) and the iPad's (`layer.rs`) are theirs: the only unsafe code of
//! the app's own, and the whole of what the platform's windowing needs.

#![allow(unsafe_code)]

use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use procraft_gpu::wgpu;
use raw_window_handle::{OhosDisplayHandle, OhosNdkWindowHandle, RawDisplayHandle, RawWindowHandle};

/// An `OHNativeWindow` the XComponent handed over, held until the render thread lets go of it.
#[derive(Clone, Copy, Debug)]
pub struct NativeWindow(pub(crate) *mut c_void);

// SAFETY: an OHNativeWindow is a handle the system owns and any thread may draw into, as the
// Android app's ANativeWindow is (see `crates/core/src/window.rs`)
unsafe impl Send for NativeWindow {}
unsafe impl Sync for NativeWindow {}

/// Windows handed over and not yet attached, by id.
static WINDOWS: Mutex<Vec<(u64, NativeWindow)>> = Mutex::new(Vec::new());
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Takes the window registered as `id`.
pub fn take(id: u64) -> Option<NativeWindow> {
    let mut windows = WINDOWS.lock().unwrap_or_else(|e| e.into_inner());
    let at = windows.iter().position(|(i, _)| *i == id)?;
    Some(windows.swap_remove(at).1)
}

/// Registers `window` (the XComponent's `OHNativeWindow`) and returns its id, for
/// [`CanvasSession::attach_window`](procraft_core::CanvasSession::attach_window): 0 if there is
/// no window.
pub fn acquire(window: *mut c_void) -> u64 {
    if window.is_null() {
        return 0;
    }
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    WINDOWS.lock().unwrap_or_else(|e| e.into_inner()).push((id, NativeWindow(window)));
    id
}

/// Lets go of a window that was never attached.
pub fn release(id: u64) {
    drop(take(id));
}

/// A wgpu surface on `window`. The caller keeps `window` alive until the surface is dropped.
pub fn create_surface(instance: &wgpu::Instance, window: &NativeWindow) -> Result<wgpu::Surface<'static>, String> {
    let Some(handle) = NonNull::new(window.0) else { return Err("no window".to_owned()) };
    let target = wgpu::SurfaceTargetUnsafe::RawHandle {
        raw_display_handle: Some(RawDisplayHandle::Ohos(OhosDisplayHandle::new())),
        raw_window_handle: RawWindowHandle::OhosNdk(OhosNdkWindowHandle::new(handle)),
    };
    // SAFETY: the handle is a live OHNativeWindow, and the render thread, which owns both, drops
    // the surface before the window
    unsafe { instance.create_surface_unsafe(target) }.map_err(|e| e.to_string())
}
