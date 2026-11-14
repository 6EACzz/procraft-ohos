//! The app itself, whatever the system it runs on: the gallery, the artwork open, the interface,
//! the input, the frames. It is written once here, against two small traits that the systems
//! answer — [`Host`](crate::host::Host) for the window it draws in, and [`Loop`] for the event
//! loop that drives it — so that the desktop (winit) and the tablet (HarmonyOS's XComponent) run
//! the same code, with the same interface, the same gestures and the same shortcuts.
//!
//! What each system answers for itself: how a window is made, how its events reach the app, and
//! how a frame is paced (see `main.rs` for the desktop, `ohos_main.rs` for the tablet).

use std::sync::Arc;
use std::time::{Duration, Instant};

use procraft_core::CanvasSession;
use winit::event::WindowEvent;

use crate::files;
use crate::gallery;
use crate::host::Host;
use crate::input;
use crate::ui::{Action, Interface};

/// How long an artwork may take to open before the interface says it is (a slow GPU's shader
/// compiler, a large artwork's layers).
pub const SLOW_OPENING: Duration = Duration::from_millis(500);

/// How long the canvases' pipelines may be made before the gallery says so.
pub const CANVASES_SLOW: Duration = Duration::from_secs(2);

/// How long a finger's touch stays where it landed with Tap Assistance on.
pub const TAP_HOLD: Duration = Duration::from_millis(300);

/// How far it may drift meanwhile and still be held (points).
pub const TAP_DRIFT: f64 = 24.0;

/// How much memory the layers' textures may take on the GPU, bytes.
pub const LAYERS_ON_GPU: u64 = 2 << 30;


/// Over the window while a slow artwork opens: veiled, saying so (and taking the presses).
pub fn opening_veil(ctx: &egui::Context) {
    veil(ctx, "Opening…");
}

/// Over the window while something slow is under way: veiled, saying `what` (and taking the
/// presses).
pub fn veil(ctx: &egui::Context, what: &str) {
    let screen = ctx.content_rect();
    egui::Area::new(egui::Id::unique("opening")).fixed_pos(screen.min).order(egui::Order::Foreground).show(ctx, |ui| {
        ui.allocate_rect(screen, egui::Sense::click_and_drag());
        ui.painter().rect_filled(screen, 0, crate::look::argb(OPENING_VEIL));
        let ink = crate::look::argb(crate::look::pc::TEXT);
        crate::look::text(
            ui.painter(),
            screen.center(),
            egui::Align2::CENTER_CENTER,
            what,
            17.0,
            crate::look::Weight::Regular,
            ink,
        );
    });
}

/// What the window says while the canvases' pipelines are made.
const GETTING_READY: &str = "Getting the canvas ready…";

/// What a canvas's veil says while it opens: while the pipelines it waits for are still being
/// made (`waiting`), that.
pub fn opening_words(waiting: bool) -> &'static str {
    if waiting { GETTING_READY } else { "Opening…" }
}

/// At the gallery's foot while the canvases' pipelines are still being made: a note saying so,
/// over the gallery, which takes its presses as ever.
pub fn getting_ready_note(ctx: &egui::Context) {
    let screen = ctx.content_rect();
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("getting ready")));
    let ink = crate::look::argb(crate::look::pc::TEXT);
    let wide = crate::look::text_width(&painter, GETTING_READY, 15.0, crate::look::Weight::Regular);
    let note = egui::Rect::from_center_size(
        egui::pos2(screen.center().x, screen.bottom() - 24.0 - 18.0),
        egui::vec2(wide + 36.0, 36.0),
    );
    painter.rect_filled(note, 18.0, crate::look::argb(crate::look::pc::BAR));
    crate::look::text(&painter, note.center(), egui::Align2::CENTER_CENTER, GETTING_READY, 15.0, crate::look::Weight::Regular, ink);
}

/// The veil's colour: the window under it darkened.
const OPENING_VEIL: u32 = 0x66000000;

/// The event loop as the app uses it: when to wake next, and when to end.
pub trait Loop {
    /// Ends the loop (the app stops).
    fn exit(&self);

    /// Waits for an event, or for nothing more to come.
    fn wait(&self);

    /// Waits for an event, or until `at`.
    fn wait_until(&self, at: Instant);
}

/// A new artwork's size when the screen's isn't known.
pub const FALLBACK_CANVAS: (u32, u32) = (1920, 1080);

/// A picture being made a new artwork: since when, the gallery folder it goes in, and its reading.
pub type ReadingPhoto = (Instant, Option<String>, std::thread::JoinHandle<Result<Arc<CanvasSession>, String>>);

/// An artwork in the gallery being opened: since when, its folder, the moving format it is
/// opened to be shared as (the gallery's Share), and its reading.
pub type ReadingArtwork =
    (Instant, String, Option<crate::animated_export::Animated>, std::thread::JoinHandle<Result<Arc<CanvasSession>, String>>);

/// Artworks being brought in into a gallery folder: one read on a thread of its own (an artwork of
/// many layers takes seconds), since when, and those still to come.
pub struct Importing {
    pub since: Instant,
    pub stage: ImportStage,
    pub folder: Option<String>,
    pub rest: std::collections::VecDeque<Vec<std::path::PathBuf>>,
}

/// An artwork coming in: read, then (a package's) its time-lapse taken into its new folder, both
/// off the window's thread (a long time-lapse is hundreds of videos, every frame decoded).
pub enum ImportStage {
    Reading(std::thread::JoinHandle<Option<(Arc<CanvasSession>, Option<std::path::PathBuf>)>>),
    TakingTimeLapse(std::thread::JoinHandle<()>, Arc<CanvasSession>, String),
}

impl ImportStage {
    pub fn is_finished(&self) -> bool {
        match self {
            ImportStage::Reading(t) => t.is_finished(),
            ImportStage::TakingTimeLapse(t, ..) => t.is_finished(),
        }
    }
}

impl Importing {
    /// The first of `artworks` (each its files) read on its own thread, into `folder`; none left,
    /// nothing.
    pub fn next(artworks: std::collections::VecDeque<Vec<std::path::PathBuf>>, folder: Option<String>) -> Option<Importing> {
        let mut rest = artworks;
        let first = rest.pop_front()?;
        let _ = &folder;
        let reading = std::thread::spawn(move || crate::files::open_artwork_files(first));
        Some(Importing { since: Instant::now(), stage: ImportStage::Reading(reading), folder, rest })
    }
}

/// The computer's memory, bytes.
#[cfg(target_os = "linux")]
pub fn memory_total() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = text.lines().find(|l| l.starts_with("MemTotal:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb * 1024)
}

#[cfg(windows)]
pub fn memory_total() -> Option<u64> {
    None
}

#[cfg(target_os = "macos")]
pub fn memory_total() -> Option<u64> {
    None
}

/// A window event, timed for the log.
pub struct Timed(pub &'static str, pub Instant);

impl Drop for Timed {
    fn drop(&mut self) {
        let took = self.1.elapsed();
        if took > Duration::from_millis(200) {
            log::warn!("{} took {} ms", self.0, took.as_millis());
        }
    }
}

/// The app, whatever the system it runs on: `H` is the window it draws in, `L` the loop that
/// drives it.
pub struct App<H: Host, L: Loop> {
    /// The loop that drives this app (its own, so that `gpu_lost` can end it).
    _loop: std::marker::PhantomData<L>,
    pub host: Option<H>,
    pub wake: Box<dyn Fn() + Send + Sync>,
    pub started: Instant,
    /// When the last frame was drawn, and when another is wanted (the canvas or the interface still
    /// moving, or a hold to be timed).
    pub last_frame: Instant,
    pub again: Option<Instant>,
    /// Since when a plugin has been finishing a fill.
    pub finishing_since: Option<Instant>,
    /// An artwork's canvas being made on a thread of its own as it opens, and since when.
    pub opening: Option<(Instant, std::thread::JoinHandle<()>)>,
    /// Artworks being brought in, read on a thread of their own one after another.
    pub importing: Option<Importing>,
    /// An artwork in the gallery being opened, read on a thread of its own (an artwork of many
    /// layers holds hundreds of MB): since when, its folder, and the reading.
    pub reading_artwork: Option<ReadingArtwork>,
    /// The gallery's animated Share of the artwork just opened, started once its canvas is made
    /// (the preview's frames and the file are read from it).
    pub share_once_made: Option<crate::animated_export::Animated>,
    /// A picture made a new artwork (the gallery's Photo), read on a thread of its own: since
    /// when, the gallery folder it goes in, and the reading.
    pub reading_photo: Option<ReadingPhoto>,
    /// The artwork open and its folder in the gallery; none while the gallery shows.
    pub session: Option<Arc<CanvasSession>>,
    pub artwork: Option<String>,
    pub gallery: gallery::Gallery,
    pub ui: Interface,
    pub input: input::Input,
    /// The window's title as last set.
    pub title: String,
    /// The gallery zooming into the artwork opening, or out of the one closing.
    pub zoom: Option<crate::zoom::Zoom>,
    /// Files dropped on the window (or the app opened with), taken in together at the next frame
    /// (a model with its material file and images comes in one drop).
    pub dropped: Vec<std::path::PathBuf>,
    /// Tap Assistance: a lone finger's touch, where it landed and when, while it is held there.
    pub tap_held: Option<(u64, winit::dpi::PhysicalPosition<f64>, Instant)>,
    /// An artwork the gallery asked to drag out, for the driver to carry (a system with a drag of
    /// its own does; the tablet shares it as a file instead).
    pub drag_out: Vec<procraft_core::GalleryArtwork>,
    /// The window was closed (the artwork open kept): the app's end needs nothing more.
    pub closed: bool,
}

impl<H: Host, L: Loop> App<H, L> {
    /// The app as it starts: the gallery, nothing open.
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> App<H, L> {
        App {
            _loop: std::marker::PhantomData,
            host: None,
            wake: Box::new(wake),
            started: Instant::now(),
            last_frame: Instant::now(),
            again: None,
            finishing_since: None,
            opening: None,
            importing: None,
            reading_artwork: None,
            share_once_made: None,
            reading_photo: None,
            session: None,
            artwork: None,
            gallery: gallery::Gallery::new(),
            ui: Interface::default(),
            input: input::Input::default(),
            title: String::new(),
            zoom: None,
            // files the app was opened with ("Open with", or dropped on its icon) come in as a drop
            // on the gallery does
            dropped: std::env::args_os().skip(1).map(std::path::PathBuf::from).filter(|p| p.is_file()).collect(),
            tap_held: None,
            drag_out: Vec::new(),
            closed: false,
        }
    }

    /// The window is ready: the interface is shown in it.
    pub fn resumed(&mut self, host: H) {
        if let Some(session) = &self.session {
            session.set_display_scale(host.scale());
        }
        host.request_redraw();
        self.host = Some(host);
    }

    /// Shows `session` in the window, in place of the artwork open.
    pub fn show(&mut self, session: Arc<CanvasSession>) {
        let Some(host) = self.host.as_mut() else { return };
        let size = host.size();
        let scale = host.scale();
        session.set_display_scale(scale);
        // the system's ask for less motion: a dropped colour's fill shows at once
        session.set_reduce_motion(crate::colour_drop::reduce_motion());
        // until it shows, a driver crashing counts against the backend (unless a canvas has drawn
        // on it before)
        crate::startup::canvas_opening();
        // its layers go up on a thread of their own (an artwork of many takes seconds), the window
        // drawing meanwhile
        let (gpu, opened) = (host.gpu().clone(), session.clone());
        self.opening = Some((Instant::now(), std::thread::spawn(move || opened.use_gpu(gpu))));
        // the brushes as the last artwork left them
        if let Some(last) = &self.session {
            last.keep_brushes(files::brushes_dir());
        }
        session.open_brushes(files::brushes_dir());
        self.ui.opened(&session, scale * host.ctx().zoom_factor());
        session.show_in(size.0, size.1);
        self.input = input::Input::default();
        // time on it counts while it is shown (not while the window is minimised)
        session.set_tracking(!host.minimised());
        self.session = Some(session);
        host.request_redraw();
    }

    /// The brush studio's drawing pad: its canvas made when the studio opens (the size the pad
    /// shows at, on this device), drawn into its picture each frame, and let go when it closes.
    /// True while the pad wants another frame.
    pub fn studio_pad(&mut self, session: &Arc<CanvasSession>) -> bool {
        let Some(host) = self.host.as_mut() else { return false };
        let studio = &mut self.ui.studio;
        if !studio.open {
            // closing the studio lets its texture go first: the picture goes whether or not
            studio.pad_texture = None;
            host.drop_pad();
            return false;
        }
        let ppp = host.ctx().pixels_per_point();
        let Some(rect) = studio.pad_rect else { return true };
        let size = ((rect.width() * ppp).round() as u32, (rect.height() * ppp).round() as u32);
        if size.0 < 2 || size.1 < 2 {
            return true;
        }
        if studio.pad.is_none() {
            // a new pad is shown afresh: the picture an earlier one was shown in (the studio closed
            // without it being let go) would be drawn into without the new pad ever being told its
            // size and view, and the pad would take no strokes
            host.drop_pad();
            let pad = CanvasSession::new(size.0, size.1);
            pad.use_gpu(host.gpu().clone());
            pad.set_display_scale(ppp);
            studio.start_pad(pad, session);
        }
        let Some(pad) = studio.pad.clone() else { return false };
        studio.pad_texture = Some(host.draw_pad(&pad, size, self.started));
        pad.wants_frame()
    }

    /// Runs the interface, draws the canvas and the interface over it, and does what was asked.
    pub fn frame(&mut self) {
        if self.host.is_none() {
            return;
        }
        // on OpenGL nothing is drawn while the canvases' pipelines are made (see
        // `Screen::gl_busy`), and nothing that would draw is taken in: looked at again soon (the
        // window answers meanwhile)
        if self.host.as_ref().is_some_and(crate::host::Host::gl_busy) {
            self.again = Some(Instant::now() + Duration::from_millis(100));
            return;
        }
        self.take_dropped();
        self.imported();
        self.artwork_read();
        self.photo_read();
        let Some(host) = self.host.as_mut() else { return };
        let Some(session) = self.session.clone() else {
            self.gallery_frame();
            return;
        };
        self.last_frame = Instant::now();
        // the canvas made: drawn from now on; a slow one says it is opening
        if self.opening.as_ref().is_some_and(|(_, made)| made.is_finished())
            && let Some((_, made)) = self.opening.take()
        {
            let _ = made.join();
        }
        // the gallery's animated Share, now there is a canvas to read the frames from
        if self.opening.is_none()
            && let Some(format) = self.share_once_made.take()
        {
            self.ui.share_animated_from_gallery(format);
        }
        let slow = self.opening.as_ref().is_some_and(|(since, _)| since.elapsed() >= SLOW_OPENING)
            || self.importing.as_ref().is_some_and(|i| i.since.elapsed() >= SLOW_OPENING);
        // the gallery stays as it is until there is a canvas to zoom into
        if self.opening.is_some()
            && let Some(zoom) = self.zoom.as_mut().filter(|z| z.opening)
        {
            zoom.hold();
        }
        // a plugin finishing a fill: after a moment it says so
        let finishing = session.pour_finishing();
        self.finishing_since = finishing.then(|| self.finishing_since.unwrap_or_else(Instant::now));
        let finishing_slowly = self.finishing_since.is_some_and(|since| since.elapsed() >= SLOW_OPENING);
        // an export shows its own card; the veil is for a slow save
        let exporting = !self.ui.export_card.busy()
            && crate::files::exporting_since().is_some_and(|since| since.elapsed() >= SLOW_OPENING);
        // a canvas opening while the pipelines are still being made waits for them: it says so
        let opening = opening_words(self.opening.is_some() && host.canvases_waited().is_some());
        host.size_interface(self.ui.plugins(), Some(session.as_ref()));
        let raw = host.take_input();
        let output = host.ctx().clone().run_ui(raw, |ui| {
            self.ui.show(ui, &session, &mut self.input);
            // opening: the gallery over it, growing from the artwork's picture
            if let Some(zoom) = self.zoom.as_ref().filter(|z| z.opening) {
                zoom.over_canvas(ui.ctx(), &mut self.gallery, &mut self.ui.look);
            }
            if slow {
                veil(ui.ctx(), opening);
            } else if exporting {
                veil(ui.ctx(), "Exporting…");
            } else if finishing_slowly {
                veil(ui.ctx(), "Finishing the fill…");
            }
        });
        let pad_soon = self.studio_pad(&session);
        let Some(host) = self.host.as_mut() else { return };
        self.again = host.draw(Some(&session), self.started, output).map(|after| Instant::now() + after);
        if pad_soon {
            self.again = Some(Instant::now());
        }
        // looked at again soon while the canvas is being made, an artwork read or saved as, or a
        // fill finished
        if self.ui.saved_as() || self.opening.is_some() || self.importing.is_some() || finishing {
            let soon = Instant::now() + Duration::from_millis(50);
            self.again = Some(self.again.map_or(soon, |due| due.min(soon)));
        }
        let edits = session.edits();
        let saved = edits == self.ui.saved_edits || session.edits_saved() == Some(edits);
        let unsaved = if saved { "" } else { " •" };
        let title = format!("{}{unsaved} — ProCraft", session.name());
        if title != self.title {
            host.set_title(&title);
            self.title = title;
        }
        // looked at again until a save has it (autosaves come in on their own)
        if !saved {
            let soon = Instant::now() + Duration::from_secs(1);
            self.again = Some(self.again.map_or(soon, |due| due.min(soon)));
        }
        self.zoom_on();
        // Project canvas shows the canvas as just drawn
        
        if std::mem::take(&mut self.ui.keep_brushes) {
            session.keep_brushes(crate::files::brushes_dir());
        }
        for action in std::mem::take(&mut self.ui.actions) {
            self.act(action, &session);
        }
    }

    /// The gallery over the whole window, and what a tap in it asked for.
    pub fn gallery_frame(&mut self) {
        let Some(host) = self.host.as_mut() else { return };
        self.last_frame = Instant::now();
        host.size_interface(self.ui.plugins(), None);
        let raw = host.take_input();
        let mut asked = None;
        let (gallery, interface) = (&mut self.gallery, &mut self.ui);
        let closing = self.zoom.as_ref().filter(|z| !z.opening);
        let reading = match (&self.importing, &self.reading_artwork, &self.reading_photo) {
            (None, None, None) => None,
            (importing, opening, photo) => Some(
                importing.as_ref().is_some_and(|i| i.since.elapsed() >= SLOW_OPENING)
                    || opening.as_ref().is_some_and(|(since, ..)| since.elapsed() >= SLOW_OPENING)
                    || photo.as_ref().is_some_and(|(since, ..)| since.elapsed() >= SLOW_OPENING),
            ),
        };
        // the canvases' pipelines still being made after a moment: said at the gallery's foot
        let waited = host.canvases_waited();
        // (on OpenGL from the first frame, which stays while they are made)
        let getting_ready = waited.is_some_and(|w| w >= CANVASES_SLOW) || host.canvases_later();
        let output = host.ctx().clone().run_ui(raw, |ui| {
            interface.style_once(ui.ctx());
            // closing: the gallery settling back into place, the canvas fading over it
            // otherwise in its place (a closing zoom cut short leaves nothing behind)
            match closing {
                Some(zoom) => zoom.over_gallery(ui.ctx()),
                None => crate::zoom::Zoom::end(ui.ctx()),
            }
            asked = gallery.show(ui.ctx(), &mut interface.look, &interface.colours);
            // a pen's buttons do nothing here, and nothing in the artwork opened next either
            crate::pen::take_button_presses();
            // a share from the gallery shows its card there
            interface.export_card.show(ui.ctx(), &mut interface.look);
            interface.plugin_alert(ui.ctx());
            if reading == Some(true) {
                opening_veil(ui.ctx());
            } else if getting_ready {
                getting_ready_note(ui.ctx());
            }
        });
        self.again = host.draw(None, self.started, output).map(|after| Instant::now() + after);
        // looked at again soon while an artwork is read, and once the pipelines have taken long
        // enough to say so (once made, the window is asked to draw again)
        let soon = match waited {
            _ if reading.is_some() => Some(Instant::now() + Duration::from_millis(50)),
            Some(waited) if !getting_ready => Some(Instant::now() + CANVASES_SLOW.saturating_sub(waited)),
            _ => None,
        };
        if let Some(soon) = soon {
            self.again = Some(self.again.map_or(soon, |due| due.min(soon)));
        }
        if self.title != "ProCraft" {
            host.set_title("ProCraft");
            self.title = "ProCraft".into();
        }
        self.zoom_on();
        // Project canvas is black while the gallery shows
        
        // while an artwork, a photo or an import is being read, the first tap stands: another
        // artwork, a new one or a photo isn't opened over it
        let busy = self.reading_artwork.is_some() || self.reading_photo.is_some() || self.importing.is_some();
        match asked {
            // read on a thread of its own; one at a time
            Some(crate::gallery::Asked::Open(dir)) => {
                if !busy {
                    self.read_artwork(dir, None);
                }
            }
            Some(crate::gallery::Asked::New(preset, folder)) => {
                if !busy {
                    let session = crate::files::new_from_preset(&preset);
                    let dir = self.gallery.store.new_folder(folder);
                    // its time-lapse made as the preset says
                    crate::files::keep_preset_time_lapse(&preset, &dir);
                    self.open_artwork(session, dir);
                    self.ui.started_new();
                }
            }
            Some(crate::gallery::Asked::ScreenSize(folder)) => {
                if !busy {
                    let (w, h) = self.gallery.screen_px;
                    let session = CanvasSession::new(w, h);
                    session.set_new_profile(procraft_core::P3.into());
                    let dir = self.gallery.store.new_folder(folder);
                    self.open_artwork(session, dir);
                    self.ui.started_new();
                }
            }
            // a plugin's kind of artwork, the screen's size, its plugin keeping it
            Some(crate::gallery::Asked::PluginKind(kind, folder)) => {
                let (w, h) = self.gallery.screen_px;
                let canvas = self.ui.plugins_shared().open_canvas(kind.clone(), w, h, None);
                match canvas {
                    Some(canvas) if !busy => {
                        let session = CanvasSession::new(w, h);
                        session.set_new_profile(procraft_core::P3.into());
                        if session.attach_plugin_canvas(kind, canvas) {
                            let dir = self.gallery.store.new_folder(folder);
                            self.open_artwork(session, dir);
                            self.ui.started_new();
                        }
                    }
                    Some(_) => {}
                    None => crate::files::say("The plugin couldn't make this artwork."),
                }
            }
            Some(crate::gallery::Asked::Share(list, format)) => {
                crate::files::share_artworks(&list, format, 100, &mut self.ui.export_card);
            }
            Some(crate::gallery::Asked::SharePdf(list, quality)) => {
                crate::files::share_artworks(&list, crate::files::Format::Pdf, quality, &mut self.ui.export_card);
            }
            Some(crate::gallery::Asked::DragOut(artwork)) => self.drag_out.push(artwork),
            // opened (read as Open reads it), then shared
            Some(crate::gallery::Asked::ShareAnimated(artwork, format)) => {
                if !busy {
                    self.read_artwork(artwork.dir, Some(format));
                }
            }
            // read on a thread of its own (a large photo takes a while); one at a time
            Some(crate::gallery::Asked::Photo(folder)) => {
                if !busy && let Some(path) = crate::files::pick_picture() {
                    let reading = std::thread::spawn(move || crate::files::picture_artwork(&path));
                    self.reading_photo = Some((Instant::now(), folder, reading));
                }
            }
            Some(crate::gallery::Asked::Import(folder)) => {
                if let Some(paths) = crate::files::pick_import() {
                    self.import(paths, folder);
                }
            }
            None => {}
        }
    }



    /// While the gallery zooms, another frame soon; once done, everything back in its place.
    pub fn zoom_on(&mut self) {
        let Some(zoom) = &self.zoom else { return };
        if zoom.done() {
            self.zoom = None;
            if let Some(host) = &self.host {
                crate::zoom::Zoom::end(host.ctx());
                host.request_redraw();
            }
        } else {
            self.again = Some(Instant::now());
        }
    }

    pub fn open_artwork(&mut self, session: Arc<CanvasSession>, dir: String) {
        self.ui.artwork_dir = Some(dir.clone());
        // kept saved as it changes, as the tablet's
        Arc::clone(&session).start_autosave(dir.clone());
        self.artwork = Some(dir);
        self.ui.path = None;
        self.ui.saved_edits = session.edits();
        self.show(session);
    }

    /// Tap Assistance: for its first moments a lone finger's touch stays where it landed (its
    /// moves held back, its lift put back there), so a tap that drifts still lands where it began.
    /// More fingers, a pen, a touch held longer, or one gone farther than a drift (a list scrolled)
    /// go on as they are. None: the event is held back.
    pub fn tap_assist(&mut self, event: WindowEvent) -> Option<WindowEvent> {
        let WindowEvent::Touch(touch) = &event else { return Some(event) };
        // (a finger may give a pressure too: the system says which it is)
        let pen = crate::pen::is_pen(touch.id).unwrap_or(touch.force.is_some());
        if !self.ui.tap_assistance() || pen {
            return Some(event);
        }
        let held = self.tap_held.filter(|(id, _, _)| *id == touch.id);
        let drift = TAP_DRIFT * self.host.as_ref().map_or(1.0, |s| s.scale() as f64);
        match touch.phase {
            // a second finger lets the first go on as it is
            winit::event::TouchPhase::Started if self.tap_held.is_some() => self.tap_held = None,
            winit::event::TouchPhase::Started => self.tap_held = Some((touch.id, touch.location, Instant::now())),
            winit::event::TouchPhase::Moved => match held {
                Some((_, at, since))
                    if since.elapsed() <= TAP_HOLD
                        && (touch.location.x - at.x).hypot(touch.location.y - at.y) <= drift =>
                {
                    return None;
                }
                Some(_) => self.tap_held = None,
                None => {}
            },
            winit::event::TouchPhase::Ended => {
                if let Some((_, at, _)) = held {
                    self.tap_held = None;
                    return Some(WindowEvent::Touch(winit::event::Touch { location: at, ..*touch }));
                }
            }
            winit::event::TouchPhase::Cancelled => self.tap_held = None,
        }
        Some(event)
    }

    /// Files dropped on the window: on the canvas its pictures are inserted; on the gallery they
    /// come in as Import brings them (an artwork, a picture, or a model with its files).
    pub fn take_dropped(&mut self) {
        let paths = std::mem::take(&mut self.dropped);
        // a plugin file is asked about wherever it lands
        let (plugins, paths): (Vec<_>, Vec<_>) = paths.into_iter().partition(|p| crate::files::is_plugin_file(p));
        if let Some(path) = plugins.first() {
            self.ui.offer_plugin_file(path);
        }
        if paths.is_empty() {
            return;
        }
        if let Some(session) = self.session.clone() {
            self.ui.dropped(&session, &paths);
            return;
        }
        self.import(paths, self.gallery.folder());
    }

    /// Files brought into the gallery (picked with Import, or dropped on it) into `folder`:
    /// palettes join the colour panel's, brushes wait for a canvas to open, and the rest come in as
    /// an artwork (a package, a PSD, a PDF, a picture, or a model with its files).
    pub fn import(&mut self, paths: Vec<std::path::PathBuf>, folder: Option<String>) {
        let (palettes, paths): (Vec<_>, Vec<_>) = paths.into_iter().partition(|p| crate::files::is_palette_file(p));
        if !palettes.is_empty() {
            let store = &self.ui.colours.palettes;
            let added = palettes.iter().filter(|p| {
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned());
                std::fs::read(p).is_ok_and(|bytes| store.import_file(bytes, name))
            });
            crate::files::say(if added.count() > 0 { "Palette added." } else { "This palette can't be read." });
        }
        let (brushes, paths): (Vec<_>, Vec<_>) = paths.into_iter().partition(|p| crate::files::is_brush_file(p));
        if !brushes.is_empty() {
            crate::files::keep_pending_brushes(&brushes);
        }
        // a model takes the files picked with it; else each is an artwork of its own, the one
        // before kept (closed) as the next comes in
        let artworks: std::collections::VecDeque<Vec<std::path::PathBuf>> =
            if paths.iter().any(|p| crate::files::is_model_file(p)) {
                [paths].into()
            } else {
                paths.into_iter().map(|p| vec![p]).collect()
            };
        // after any already coming in
        match &mut self.importing {
            Some(importing) => importing.rest.extend(artworks),
            None => self.importing = Importing::next(artworks, folder),
        }
    }

    /// Starts reading the artwork in gallery folder `dir` on a thread of its own, to open it (to
    /// share it as `shared_as`, if given).
    pub fn read_artwork(&mut self, dir: String, shared_as: Option<crate::animated_export::Animated>) {
        let folder = dir.clone();
        let reading = std::thread::spawn(move || CanvasSession::open(folder).map_err(|e| e.to_string()));
        self.reading_artwork = Some((Instant::now(), dir, shared_as, reading));
    }

    /// The artwork opened from the gallery, once read: shown, the gallery zooming into its
    /// picture (unless another was opened meanwhile); or, opened to be shared, shared.
    pub fn artwork_read(&mut self) {
        let Some((_, dir, shared_as, reading)) = self.reading_artwork.take_if(|(.., r)| r.is_finished()) else {
            return;
        };
        let read = reading.join().unwrap_or_else(|_| Err("it stopped".to_owned()));
        if self.session.is_some() {
            return;
        }
        match read {
            Ok(session) if shared_as.is_some() => {
                self.open_artwork(session, dir);
                self.share_once_made = shared_as;
            }
            Ok(session) => {
                let cell = self.gallery.picture_of(&dir);
                self.open_artwork(Arc::clone(&session), dir);
                // the gallery zooms into its picture
                if let (Some(cell), Some(host)) = (cell, &self.host) {
                    let size = host.size();
                    let scale = host.scale() * host.ctx().zoom_factor();
                    self.zoom = Some(crate::zoom::Zoom::opening(&session, cell, (size.0, size.1), scale));
                }
            }
            Err(e) => crate::files::tell("Can't open the artwork", &e),
        }
    }

    /// The gallery's Photo, once read: a new artwork of the picture, opened (unless another was
    /// opened meanwhile).
    pub fn photo_read(&mut self) {
        let Some((_, folder, reading)) = self.reading_photo.take_if(|(.., r)| r.is_finished()) else { return };
        let read = reading.join().unwrap_or_else(|_| Err("it stopped".to_owned()));
        if self.session.is_some() {
            return;
        }
        match read {
            Ok(session) => {
                let dir = self.gallery.store.new_folder(folder);
                self.open_artwork(session, dir);
                self.ui.started_new();
            }
            Err(e) => crate::files::tell("Can't open the picture", &e),
        }
    }

    /// An artwork read meanwhile, opened (the one open before kept), and the next one read.
    pub fn imported(&mut self) {
        let Some(importing) = self.importing.take_if(|i| i.stage.is_finished()) else { return };
        let Importing { since, stage, folder, rest } = importing;
        let (session, dir) = match stage {
            ImportStage::Reading(reading) => {
                let Ok(Some((session, package))) = reading.join() else {
                    self.importing = Importing::next(rest, folder);
                    return;
                };
                if self.session.is_some() && !self.leave_artwork() {
                    return;
                }
                let dir = self.gallery.store.new_folder(folder.clone());
                // a package's time-lapse taken in on its own thread first, the window veiled
                if let Some(package) = package {
                    let into = dir.clone();
                    let taking = std::thread::spawn(move || crate::timelapse::take_package(&package, &into));
                    let stage = ImportStage::TakingTimeLapse(taking, session, dir);
                    self.importing = Some(Importing { since, stage, folder, rest });
                    return;
                }
                (session, dir)
            }
            ImportStage::TakingTimeLapse(taking, session, dir) => {
                let _ = taking.join();
                (session, dir)
            }
        };
        // a model is a new artwork painting on it
        let model = session.model_info().is_some();
        self.open_artwork(session, dir);
        if model {
            self.ui.started_new();
        }
        self.importing = Importing::next(rest, folder);
    }



    /// The graphics device was lost (its driver reset or stopped), and all it held: nothing is
    /// saved from it (the gallery keeps the artwork as it was last saved, seconds before); the
    /// windows go, and the app says so and starts again.
    pub fn gpu_lost(&mut self, driver: &L) {
        if let Some(session) = self.session.take() {
            session.stop_autosave();
        }
        // nothing they show can be drawn (and with the screen gone this happens once)
        self.host = None;
        // as a backend starts, or a canvas first draws on it, that counts against the backend
        if let Some(tried) = crate::startup::device_lost() {
            log::warn!("the graphics device stopped as this backend started: starting again on the next");
            if !crate::startup::start_on_next(tried) {
                crate::startup::stopped("the graphics device stopped, and it couldn't start itself again");
            }
        } else {
            crate::files::tell("The graphics device stopped", "ProCraft starts again, with the artwork as it was last saved.");
            crate::startup::start_again(None);
        }
        driver.exit();
    }

    /// The canvases' pipelines are made, `fine` without a GPU error. A backend that fails on them
    /// gives way to the next: the app starts again on it while nothing is open or being read
    /// (else the next start takes it), unless it is the last there is (then kept as it is).
    pub fn canvases_made(&mut self, fine: bool, driver: &L) {
        let Some((tried, last)) = self.host.as_ref().map(|s| s.tried()) else { return };
        crate::startup::gpu_prepared(fine || last);
        if fine || last {
            return;
        }
        let busy = self.session.is_some()
            || self.reading_artwork.is_some()
            || self.reading_photo.is_some()
            || self.importing.is_some()
            || crate::files::still_writing();
        if busy || !crate::startup::kept() {
            log::warn!("the canvases' pipelines failed on this graphics backend: the next start takes the next");
            return;
        }
        log::warn!("the canvases' pipelines failed on this graphics backend: starting again on the next");
        self.host = None;
        self.closed = true;
        if !crate::startup::start_again(Some(tried)) {
            crate::startup::stopped("its pipelines failed on this graphics backend, and it couldn't start itself again");
        }
        driver.exit();
    }

    /// The stack the open artwork is in (its folder in the gallery's), none at the top.
    pub fn artwork_folder(&self) -> Option<String> {
        let folder = self.artwork.as_deref().and_then(|d| std::path::Path::new(d).parent());
        folder.map(|f| f.to_string_lossy().into_owned()).filter(|f| *f != self.gallery.store.root())
    }

    /// The window goes from the screen while the artwork open is still being made (its canvas
    /// waits for the canvases' pipelines: a minute or more on a slow GPU's shader compiler), as
    /// keeping it waits for that: no window stands still meanwhile. True if it went.
    pub fn hide_while_opening(&self) -> bool {
        let waits = self.opening.as_ref().is_some_and(|(_, made)| !made.is_finished());
        if waits && let Some(host) = &self.host {
            log::info!(
                "closing while a canvas is still being made: the window goes, the artwork is kept once it is made"
            );
            host.set_visible(false);
        }
        waits
    }

    /// The artwork open is put away and kept; false if it couldn't be.
    pub fn leave_artwork(&mut self) -> bool {
        if let Some(session) = self.session.clone() {
            // its time counted up to now, saved with it
            session.set_tracking(false);
            self.ui.closing(&session);
            // no save of its own after this one
            session.stop_autosave();
        }
        let kept = self.keep_artwork();
        // still open: kept saved again
        if !kept && let (Some(session), Some(dir)) = (&self.session, &self.artwork) {
            session.set_tracking(true);
            Arc::clone(session).start_autosave(dir.clone());
        }
        kept
    }

    /// Saves the artwork open into its folder in the gallery (and the brushes as it left them);
    /// false if it couldn't be.
    pub fn keep_artwork(&mut self) -> bool {
        let (Some(session), Some(dir)) = (self.session.clone(), self.artwork.clone()) else { return true };
        // its canvas made first (it is being made on a thread of its own), so the save has its picture
        if let Some((_, made)) = self.opening.take() {
            let _ = made.join();
        }
        session.keep_brushes(crate::files::brushes_dir());
        let (brush, mask) = self.ui.colours.brush_and_mask();
        crate::files::keep_colours(brush, mask);
        let edits = session.edits();
        match session.save(dir) {
            Ok(()) => {
                self.ui.saved_edits = edits;
                true
            }
            Err(e) => {
                crate::files::tell("Can't keep the artwork", &e.to_string());
                false
            }
        }
    }

    pub fn act(&mut self, action: Action, session: &Arc<CanvasSession>) {
        let opened = match action {
            Action::Gallery => {
                if self.leave_artwork() {
                    // the gallery zooms out of the artwork's picture
                    if let (Some(dir), Some(host)) = (&self.artwork, &self.host)
                        && let Some(cell) = self.gallery.picture_of(dir)
                    {
                        self.zoom = Some(crate::zoom::Zoom::closing(host.ctx(), session, cell));
                    }
                    self.session = None;
                    self.artwork = None;
                    self.gallery.refresh();
                    if let Some(host) = &self.host {
                        host.request_redraw();
                    }
                }
                None
            }
            // a new artwork the size of this one beside it in the gallery
            Action::New => {
                if !self.leave_artwork() {
                    return;
                }
                let info = session.info();
                let dir = self.gallery.store.new_folder(self.artwork_folder());
                self.open_artwork(CanvasSession::new(info.width, info.height), dir);
                self.ui.started_new();
                None
            }
            // one picked, beside it: read on a thread of its own as the gallery's Import does (a
            // big file takes seconds), this one open meanwhile and kept as that one opens
            Action::Open => {
                if let Some(paths) = crate::files::pick_artwork_files(&[]) {
                    let folder = self.artwork_folder();
                    match &mut self.importing {
                        Some(importing) => importing.rest.push_back(paths),
                        None => self.importing = Importing::next([paths].into(), folder),
                    }
                }
                None
            }
            Action::Save => {
                self.keep_artwork();
                None
            }
            Action::SaveAs => {
                crate::files::save(session, &mut self.ui, true);
                None
            }
            Action::Export(format, pdf_quality) => {
                crate::files::export(session, format, self.ui.artwork_dir.as_deref(), pdf_quality, &mut self.ui.export_card);
                None
            }
            // the file a plugin makes of the artwork's layers, by its place among those it offers
            Action::PluginExport(index) => {
                if let Some(export) = session.plugin_exports().into_iter().nth(usize::from(index)) {
                    crate::files::export_plugin(session, &export.key, &mut self.ui.export_card);
                }
                None
            }
        };
        if let Some(session) = opened {
            self.show(session);
        }
    }
}
