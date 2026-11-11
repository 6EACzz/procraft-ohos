//! The ProCraft interface, written once for every system the app runs on: the desktop's (see
//! `main.rs`, on winit) and the tablet's (see `ohos_main.rs`, on HarmonyOS's XComponent). The
//! window is the platform's (`host.rs`), the rest is this.

pub mod actions;
pub mod adjust_panels;
pub mod adjustments;
pub mod alert;
pub mod animated_export;
pub mod animation;
pub mod canvas_info;
pub mod chrome;
pub mod colour_drop;
pub mod colour_models;
pub mod colours;
pub mod compact_colours;
pub mod companion;
pub mod copy_paste;
pub mod crop;
pub mod curve;
pub mod custom_canvas;
pub mod export_card;
pub mod files;
#[cfg(target_os = "macos")]
pub mod finder;
pub mod gallery;
pub mod gesture_controls;
pub mod gradient_map;
pub mod guide_editor;
pub mod icons;
pub mod input;
pub mod layer_choice;
pub mod lang;
pub mod layers;
pub mod library;
pub mod licences;
pub mod lighting;
pub mod liquify;
pub mod look;
pub mod menu;
pub mod model_picture;
pub mod overlays;
pub mod panel;
pub mod pdf;
pub mod pen;
pub mod prefs;
pub mod projection;
pub mod quick_menu;
pub mod recolor;
pub mod reference;
pub mod screen;
pub mod selection;
pub mod settings;
pub mod shortcuts;
pub mod sound;
pub mod source_editor;
pub mod startup;
pub mod studio;
pub mod system_clipboard;
pub mod text;
pub mod timelapse;
pub mod tool_header;
pub mod transform;
pub mod transform_numeric;
pub mod ui;
pub mod video;
pub mod zoom;

pub mod app;
pub mod host;

#[cfg(target_env = "ohos")]
pub mod ohos_files;
#[cfg(target_env = "ohos")]
pub mod ohos_input;
#[cfg(target_env = "ohos")]
pub mod ohos_main;
#[cfg(target_env = "ohos")]
pub mod ohos_platform;
#[cfg(target_env = "ohos")]
pub mod ohos_screen;
#[cfg(target_env = "ohos")]
pub mod ohos_window;

pub mod platform;

/// The app's icon, for the window's title bar and the task bar.
pub fn window_icon() -> Option<winit::window::Icon> {
    let picture = image::load_from_memory(include_bytes!("../../assets/app-icon/procraft-256.png")).ok()?;
    let rgba = picture.into_rgba8();
    let (width, height) = rgba.dimensions();
    winit::window::Icon::from_rgba(rgba.into_raw(), width, height).ok()
}

/// How long one frame may take on this screen: a display's refresh, as its monitor says.
pub fn frame_gap(window: &winit::window::Window) -> std::time::Duration {
    let millihertz = window.current_monitor().and_then(|m| m.refresh_rate_millihertz()).filter(|&m| m > 0);
    std::time::Duration::from_secs_f64(1000.0 / f64::from(millihertz.unwrap_or(60_000)))
}
