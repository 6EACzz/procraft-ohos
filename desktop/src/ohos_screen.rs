//! The window HarmonyOS gives the app (an XComponent's `OHNativeWindow`), the wgpu surface on it,
//! and the interface's own state: the egui context and the renderer that draws it. This is the
//! tablet's `screen.rs`: where the desktop's window is winit's, the tablet's is the surface the
//! XComponent hands over, and where the desktop's egui input comes from `egui_winit`, the
//! tablet's comes from its touches and keys (see `ohos_input.rs`).
//!
//! Everything above this — the gallery, the canvas's bars, the panels, the studio, the shortcuts
//! — is the desktop's own, unchanged: the same `Interface`, the same `Gallery`, the same input
//! state machine, given the same events.

use std::time::{Duration, Instant};

use procraft_core::{CanvasSession, Plugins, configure, screen_profile};
use procraft_gpu::{Gpu, wgpu};

use crate::ohos_window::NativeWindow;

/// The window's surface and the device the canvas and the interface share, as the desktop's
/// `Screen`, on the surface the XComponent handed over.
pub struct OhosScreen {
    surface: wgpu::Surface<'static>,
    config: Option<wgpu::SurfaceConfiguration>,
    pub gpu: Gpu,
    pub ctx: egui::Context,
    /// Made for the surface's format.
    renderer: egui_wgpu::Renderer,
    format: wgpu::TextureFormat,
    /// The brush studio's drawing pad: its own picture, shown by the interface.
    pad: Option<Pad>,
    /// The window's size in pixels, as the surface is configured with.
    size: (u32, u32),
    /// Pixels per point: the interface's scale (a tablet's 2.0 or more).
    pixels_per_point: f32,
    /// When the interface began (egui's clock).
    began: Instant,
    /// The modifiers as they were handed over last frame (said again only when they change).
    modifiers: egui::Modifiers,
    /// The canvases' pipelines being made on a thread of their own (seconds): whether they were
    /// made without a GPU error.
    canvases: Option<std::thread::JoinHandle<bool>>,
    canvases_since: Instant,
    /// On GL only (see [`make_canvases`]): how the pipelines are made once the first frame has
    /// shown, kept until then. A thread making them holds the same GL context the frame is drawn
    /// with, so the two must not run at once.
    canvases_later: Option<Box<dyn Fn() -> Option<std::thread::JoinHandle<bool>> + Send>>,
    /// Whether the soft keyboard was asked for last frame (it is asked for only when that
    /// changes: an ask each frame would queue a keyboard each frame).
    typing: bool,
}

/// A canvas drawn into a picture of its own (the surface's format), its size, and the
/// interface's name for it.
struct Pad {
    view: wgpu::TextureView,
    size: (u32, u32),
    id: egui::TextureId,
}

/// The stack the canvases' pipelines are made on (reserved; only what is used is taken).
const PIPELINES_STACK: usize = 64 << 20;

/// Whether the window's surface may be wide colour: no, as egui draws the interface in sRGB onto
/// it as it is (as the desktop's).
const INTERFACE_WIDE: bool = false;

/// Makes the canvases' pipelines on `gpu` (and the brush images they hold) on a thread of their
/// own, woken once they are; a canvas made meanwhile waits for them.
fn make_canvases(gpu: &Gpu, wake: impl Fn() + Send + 'static) -> Option<std::thread::JoinHandle<bool>> {
    let gpu = gpu.clone();
    // on a big stack: a driver's shader compiler recurses, and a stack overflow ends the app with
    // next to no word
    let thread = std::thread::Builder::new().name("procraft-pipelines".to_owned()).stack_size(PIPELINES_STACK);
    let made = thread.spawn(move || {
        log::info!("making the canvases' pipelines");
        let (started, errors) = (Instant::now(), gpu.errors());
        let fine = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| gpu.prepare_canvases())) {
            Ok(()) => gpu.errors() == errors && !gpu.is_lost(),
            Err(panic) => {
                log::error!("the canvases' pipelines panicked: {}", procraft_gpu::panic_message(panic.as_ref()));
                false
            }
        };
        log::info!(
            "the canvases' pipelines made in {} ms: {} GPU errors, lost {}",
            started.elapsed().as_millis(),
            gpu.errors().saturating_sub(errors),
            gpu.is_lost()
        );
        wake();
        fine
    });
    made.inspect_err(|e| log::warn!("the canvases' pipelines' thread: {e}")).ok()
}

impl OhosScreen {
    /// The window's surface, and a device that can show it: the first of the backends in `order`
    /// (see `Gpu::instance_after`), from backend `from` on (round to those before it), that makes
    /// both and can show on the window without an error. `trying` is told each backend before it
    /// is tried. The canvases' pipelines are then made on a thread of their own, `wake` woken
    /// once they are.
    pub fn new(
        window: &NativeWindow,
        size: (u32, u32),
        pixels_per_point: f32,
        order: &[usize],
        from: usize,
        mut trying: impl FnMut(usize),
        wake: impl Fn() + Send + Clone + 'static,
    ) -> Result<(OhosScreen, usize), String> {
        let size = (size.0.max(1), size.1.max(1));
        let count = order.len().max(1);
        let mut said = Vec::new();
        let mut made = None;
        for tried in 0..count {
            let at = (from + tried) % count;
            let backend = order.get(at).copied().unwrap_or(at);
            trying(backend);
            let Some(instance) = Gpu::instance_after(backend) else { continue };
            let surface = match crate::ohos_window::create_surface(&instance, window) {
                Ok(s) => s,
                Err(e) => {
                    said.push(format!("backend {backend}: {e}"));
                    continue;
                }
            };
            let gpu = match Gpu::for_surface(instance, &surface) {
                Ok(g) => g,
                Err(e) => {
                    said.push(format!("backend {backend}: {e}"));
                    continue;
                }
            };
            let config = configure(&gpu, &surface, size, INTERFACE_WIDE);
            if config.is_none() {
                said.push(format!("backend {backend}: the surface couldn't be set up"));
                continue;
            }
            made = Some((surface, gpu, config));
            break;
        }
        let Some((surface, gpu, config)) = made else {
            return Err(if said.is_empty() {
                "no graphics backend could make a device".to_owned()
            } else {
                said.join("; ")
            });
        };
        let format = config.as_ref().map_or(wgpu::TextureFormat::Bgra8Unorm, |c| c.format);
        let renderer = egui_wgpu::Renderer::new(&gpu.device, format, egui_wgpu::RendererOptions::default());
        let ctx = egui::Context::default();
        // Ctrl (⌘) with + - 0 zoom the canvas (the interface's shortcuts), not the whole interface
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        // the page's density is egui's *native* one: its own zoom stays 1.0, and a point is
        // `pixels_per_point` pixels (only the viewport's info can say so, not the zoom)
        ctx.set_zoom_factor(1.0);
        // on GL the pipelines' thread and the frame share the device's context, which wgpu-hal
        // lets one of them hold at a time (it gives up after six seconds and ends the app): the
        // desktop makes them from the frame that first showed, never beside one, and so does this
        let later: Option<Box<dyn Fn() -> Option<std::thread::JoinHandle<bool>> + Send>> = if gpu.is_gl() {
            let (gpu, wake) = (gpu.clone(), wake.clone());
            Some(Box::new(move || make_canvases(&gpu, wake.clone())))
        } else {
            None
        };
        let canvases = match &later {
            Some(_) => None,
            None => make_canvases(&gpu, wake),
        };
        Ok((
            OhosScreen {
                surface,
                config,
                gpu,
                ctx,
                renderer,
                format,
                pad: None,
                size,
                pixels_per_point,
                began: Instant::now(),
                modifiers: egui::Modifiers::default(),
                canvases,
                canvases_since: Instant::now(),
                canvases_later: later,
                typing: false,
            },
            0,
        ))
    }

    /// Whether the canvases' pipelines were made without a GPU error, once they are made (told
    /// once only). Not while they are still being made: the frame carries on (the thread would
    /// hold the GL context the frame needs).
    pub fn canvases_made(&mut self) -> Option<bool> {
        let made = self.canvases.as_ref()?;
        if !made.is_finished() {
            return None;
        }
        Some(self.canvases.take()?.join().unwrap_or(false))
    }

    /// Since when the canvases' pipelines have been being made, while they are.
    pub fn canvases_waited(&self) -> Option<Duration> {
        self.canvases.as_ref().map(|_| self.canvases_since.elapsed())
    }

    /// Whether the canvases' pipelines are made from the first frame that shows, rather than from
    /// [`OhosScreen::new`] (on GL: see the note there).
    pub fn canvases_later(&self) -> bool {
        self.canvases_later.is_some()
    }

    /// Whether a thread is making the canvases' pipelines this frame can't be drawn beside (it
    /// holds the GL context the frame would draw with).
    pub fn gl_busy(&self) -> bool {
        self.gpu.is_gl() && self.canvases.as_ref().is_some_and(|made| !made.is_finished())
    }

    pub fn pixels_per_point(&self) -> f32 {
        self.pixels_per_point
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// The window's size changed (the XComponent turned, or its panel did).
    pub fn resize(&mut self, width: u32, height: u32) {
        let size = (width.max(1), height.max(1));
        if size == self.size && self.config.is_some() {
            return;
        }
        self.size = size;
        self.config = configure(&self.gpu, &self.surface, size, INTERFACE_WIDE);
    }

    /// The interface's scale (the page's own density: its points to pixels), as the shell says.
    /// It is egui's *native* scale (its zoom stays 1.0, as the desktop's does), so it is handed
    /// over with the viewport's own info each frame, never as a zoom.
    pub fn set_pixels_per_point(&mut self, pixels_per_point: f32) {
        if !(pixels_per_point > 0.1) {
            return;
        }
        self.pixels_per_point = pixels_per_point;
    }

    /// Hands the interface the plugins' sizes (as the desktop's `Screen::size_interface`).
    pub fn size_interface(&self, plugins: &Plugins, session: Option<&CanvasSession>) {
        let ctx = &self.ctx;
        let scale = self.pixels_per_point;
        let (width, height) = self.size;
        let shown = plugins.interface_scale(width as f32 / scale, height as f32 / scale);
        if ctx.zoom_factor() != shown {
            ctx.request_repaint();
            if ctx.input(|i| i.pointer.any_down()) {
                return;
            }
            // at once, not from the next frame on: this one's input is about to be read at it
            ctx.options_mut(|o| o.zoom_factor = shown);
        }
        if let Some(session) = session {
            session.set_interface_size(shown);
        }
    }

    /// Whether `p` (window pixels, as the canvas's own input is) is on the interface rather than
    /// the canvas, which shows in `canvas` (points): outside the canvas at all, or under a panel or
    /// a popover drawn over it (anything above the background layer, as the desktop's own says it).
    pub fn over_interface(&self, p: (f32, f32), canvas: egui::Rect) -> bool {
        // egui lays itself out in points, so the window's pixels become points here (as the
        // desktop's `Screen::over_interface` does)
        let at = egui::pos2(p.0, p.1) / self.ctx.pixels_per_point();
        !canvas.contains(at) || self.ctx.layer_id_at(at).is_some_and(|l| l.order != egui::Order::Background)
    }

    /// The pad's picture for the studio, drawn fresh.
    pub fn draw_pad(&mut self, pad: &CanvasSession, size: (u32, u32), started: Instant) -> egui::TextureId {
        let size = (size.0.max(1), size.1.max(1));
        let config = self.config.as_ref();
        let screen = config.map_or(procraft_engine::profile::Profile::Srgb, screen_profile);
        if self.pad.as_ref().is_none_or(|p| p.size != size) {
            let device = &self.gpu.device;
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("drawing pad"),
                size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let shown = &view;
            let id = match &self.pad {
                Some(p) => {
                    self.renderer.update_egui_texture_from_wgpu_texture(device, shown, wgpu::FilterMode::Linear, p.id);
                    p.id
                }
                None => self.renderer.register_native_texture(device, shown, wgpu::FilterMode::Linear),
            };
            pad.show_in(size.0, size.1);
            self.pad = Some(Pad { view, size, id });
        }
        let Some(p) = &self.pad else { return egui::TextureId::default() };
        pad.draw(&p.view, self.format, size, screen, started);
        p.id
    }

    /// The pad's picture goes (the studio closed).
    pub fn drop_pad(&mut self) {
        if let Some(p) = self.pad.take() {
            self.renderer.free_texture(&p.id);
        }
    }

    /// The egui input for this frame: the interface's size, time, and the events the platform
    /// gathered since the last frame (`ohos_input::Input`).
    pub fn take_input(&mut self, mut events: Vec<egui::Event>, modifiers: egui::Modifiers, focused: bool) -> egui::RawInput {
        let now = Instant::now();
        let points = (self.size.0 as f32 / self.pixels_per_point, self.size.1 as f32 / self.pixels_per_point);
        // the modifiers held, as egui wants them each frame
        // only when they changed: an event each frame is an interface that never settles, and
        // egui would take every frame as one with something to answer
        if modifiers != self.modifiers {
            self.modifiers = modifiers;
            events.push(egui::Event::ModifiersChanged(modifiers));
        }
        let mut raw = egui::RawInput {
            viewport_id: egui::ViewportId::ROOT,
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(points.0, points.1))),
            max_texture_side: Some(usize::try_from(self.gpu.max_texture_size()).unwrap_or(8192)),
            time: Some(now.duration_since(self.began).as_secs_f64()),
            // how long this frame is expected to last, for the animations that go by it (egui's
            // own guess, a sixtieth of a second: left as it is, as the desktop leaves it — nought
            // would tell every animation it has no time to move in, and none would ever settle)
            predicted_dt: 1.0 / 60.0,
            events,
            focused,
            ..Default::default()
        };
        // the window's own size, which the frame is drawn at (a default viewport would have egui
        // guess a size of its own, and lay the interface out for it), and the page's own density
        let rect = raw.screen_rect.unwrap_or(egui::Rect::ZERO);
        let mut info = egui::ViewportInfo::default();
        info.native_pixels_per_point = Some(self.pixels_per_point);
        info.inner_rect = Some(rect);
        info.outer_rect = Some(rect);
        info.focused = Some(focused);
        raw.viewports.insert(egui::ViewportId::ROOT, info);
        raw
    }

    /// What the interface asked the platform to do this frame, done: the clipboard written to
    /// (a copy, a cut, a paste of what the shell holds), a link opened, the soft keyboard raised or
    /// put away. The desktop's `handle_platform_output` (`egui_winit`) does the same for its window.
    fn answer_platform_output(&mut self, output: &egui::PlatformOutput) {
        crate::ohos_input::set_cursor(output.cursor_icon);
        for command in &output.commands {
            match command {
                // a copy or a cut: what the interface selected, as text
                egui::OutputCommand::CopyText(text) => crate::platform::clipboard_write_text(text),
                // (the desktop keeps a primary selection beside the clipboard; a tablet has none)
                egui::OutputCommand::TextSelectionSettled(_) => {}
                // a picture copied out of a canvas (the layers' own pictures, a screenshot of one)
                egui::OutputCommand::CopyImage(image) => {
                    let (width, height) = (image.size[0] as u32, image.size[1] as u32);
                    crate::platform::clipboard_write_picture(width, height, image.as_raw());
                }
                // a link the interface shows: nothing to open it with here, and the app's own
                // links are opened by the shell when it is told to
                egui::OutputCommand::OpenUrl(url) => log::info!("a link was asked for: {}", url.url),
            }
        }
        // a field is being typed in: the soft keyboard rises for it, and goes when it is done
        // (egui says so with the input method it wants: see `TextEdit`'s own `o.ime`)
        let typing = output.ime.is_some();
        if typing != self.typing {
            self.typing = typing;
            crate::ohos_platform::keyboard(typing);
        }
    }

    /// Draws a frame: the canvas (if one is open) and the interface over it.
    pub fn draw(
        &mut self,
        session: Option<&CanvasSession>,
        started: Instant,
        output: egui::FullOutput,
    ) -> Option<Duration> {
        let egui::FullOutput { platform_output, mut textures_delta, shapes, pixels_per_point: _, viewport_output } =
            output;
        self.answer_platform_output(&platform_output);
        let interface_after = viewport_output
            .get(&egui::ViewportId::ROOT)
            .map(|v| v.repaint_delay)
            .filter(|d| *d < Duration::from_secs(60 * 60));
        let (device, queue) = (&self.gpu.device, &self.gpu.queue);
        for (id, deltas) in std::mem::take(&mut textures_delta.set) {
            for delta in &deltas {
                self.renderer.update_texture(device, queue, id, delta);
            }
        }
        for id in std::mem::take(&mut textures_delta.free) {
            self.renderer.free_texture(&id);
        }
        let Some(config) = self.config.as_ref() else { return None };
        let size = (config.width, config.height);
        let (format, screen) = (config.format, screen_profile(config));
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.config = configure(&self.gpu, &self.surface, size, INTERFACE_WIDE);
                return Some(Duration::from_millis(4));
            }
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Validation => return Some(Duration::from_millis(16)),
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let canvas_after = session.and_then(|s| s.draw(&view, format, size, screen, started));
        let (device, queue) = (&self.gpu.device, &self.gpu.queue);
        // egui's own scale, as the input was read at (never changed here: the platform sets it)
        // egui's own scale, as the interface was laid out at
        let ppp = self.ctx.pixels_per_point();
        let jobs = self.ctx.tessellate(shapes, ppp);
        let screen_desc = egui_wgpu::ScreenDescriptor { size_in_pixels: [size.0, size.1], pixels_per_point: ppp };
        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("interface") });
        let uploads = self.renderer.update_buffers(device, queue, &mut encoder, &jobs, &screen_desc);
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("interface"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    // over the canvas just drawn (the gallery covers the whole window)
                    ops: wgpu::Operations {
                        load: if session.is_some() {
                            wgpu::LoadOp::Load
                        } else {
                            wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.renderer.render(&mut pass.forget_lifetime(), &jobs, &screen_desc);
        }
        queue.submit(uploads.into_iter().chain([encoder.finish()]));
        queue.present(frame);
        // the start's own record (the desktop's `Screen::draw` keeps it the same way): the window
        // shows a frame, so it draws; and the canvases' pipelines are made from here on, now that
        // this frame is done with the context they would hold
        crate::startup::gpu_shown();
        if let Some(later) = self.canvases_later.take() {
            self.canvases = later();
            self.canvases_since = Instant::now();
        }
        if crate::startup::canvas_awaited() {
            crate::startup::canvas_shown(session.map(|s| s.frames_drawn() > 0));
        }
        match (canvas_after, interface_after) {
            (Some(canvas), Some(interface)) => Some(canvas.min(interface)),
            (canvas, interface) => canvas.or(interface),
        }
    }
}
