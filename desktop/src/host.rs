//! What the app asks of the window it draws in, whatever the system that made it: the desktop's
//! winit window, or the tablet's XComponent surface. Everything above — the gallery, the canvas's
//! bars, the panels, the studio, the shortcuts — is written once against this, so that both run
//! the same code.
//!
//! The interface draws itself into whatever surface this hands it: `draw` takes the frame's
//! `egui::FullOutput` and puts it on the window (the canvas under it, when one is open).

use std::time::{Duration, Instant};

use procraft_core::CanvasSession;

/// A window the app draws in, as the app needs it: its size and scale, the interface's own state
/// (egui's context and renderer), the canvas's drawing, and the frames it asks for.
pub trait Host {
    /// The window's size in pixels.
    fn size(&self) -> (u32, u32);

    /// Pixels per point: the interface's scale.
    fn scale(&self) -> f32;

    /// The interface's context (the gallery, the canvas's bars and the rest are drawn into it).
    fn ctx(&self) -> &egui::Context;

    /// The GPU the canvas and the interface are drawn with.
    fn gpu(&self) -> &procraft_gpu::Gpu;

    /// Asks for a frame (a touch came, the interface asked for another).
    fn request_redraw(&self);

    /// The window's title, for a system that shows one.
    fn set_title(&self, title: &str);

    /// The window's icon, or nothing on a system without one.
    fn window_icon(&self) -> Option<winit::window::Icon>;

    /// The screen's refresh, as the frame's pace (a hold to be timed, a canvas settling).
    fn frame_gap(&self) -> Duration;

    /// Whether the window is minimised (nothing shows until it isn't).
    fn minimised(&self) -> bool;

    /// The interface's input for this frame: its size, time and the events gathered since the
    /// last one.
    fn take_input(&mut self) -> egui::RawInput;

    /// Draws the frame: the canvas (if one is open) and the interface over it. How soon another
    /// is wanted, if one is.
    fn draw(&mut self, session: Option<&CanvasSession>, started: Instant, output: egui::FullOutput) -> Option<Duration>;

    /// The interface's scale changed (a system setting, or the interface's own zoom).
    fn set_pixels_per_point(&mut self, pixels_per_point: f32);

    /// Hands the interface the plugins' sizes.
    fn size_interface(&self, plugins: &procraft_core::Plugins, session: Option<&CanvasSession>);

    /// Whether `p` (points) is over the interface rather than the canvas, which shows in `canvas`.
    fn over_interface(&self, p: (f32, f32), canvas: egui::Rect) -> bool;

    /// The brush studio's drawing pad, drawn into a picture of its own.
    fn draw_pad(&mut self, pad: &CanvasSession, size: (u32, u32), started: Instant) -> egui::TextureId;

    /// The pad's picture goes (the studio closed).
    fn drop_pad(&mut self);

    /// Whether the canvases' pipelines were made without a GPU error, once they are made.
    fn canvases_made(&mut self) -> Option<bool>;

    /// Since when the canvases' pipelines have been being made, while they are.
    fn canvases_waited(&self) -> Option<Duration>;

    /// On OpenGL they are made only after the first frame, and nothing is drawn while they are.
    fn gl_busy(&self) -> bool;

    /// Whether the canvases' pipelines are to be made once the window's first frame shows (on
    /// OpenGL): nothing is drawn while they are.
    fn canvases_later(&self) -> bool;

    /// The backends tried before this window's, and whether this is the last there is (a backend
    /// that fails on the canvases' pipelines gives way to the next).
    fn tried(&self) -> (usize, bool);

    /// The window is shown or hidden (while a slow artwork opens, or as it ends).
    fn set_visible(&self, shown: bool);
}
