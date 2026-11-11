//! ProCraft on the tablet (HarmonyOS): the app's own interface (`app.rs`, the same one the
//! desktop runs) on the window an XComponent hands over, driven by the system's touches, its pen
//! and its keys.
//!
//! The shell — the ArkTS page that shows the XComponent — starts this once the surface exists,
//! then hands over what the system gives it (touches, keys, a size change, the window going away)
//! through the C functions at the foot of this file. Everything else is the desktop's: the
//! gallery, the canvas's bars, the panels, the studio, the gestures, the shortcuts.
//!
//! One thread runs the app: it waits for a wake (a touch, a key, the interface asking for a frame)
//! and draws one, as the desktop's winit loop does.

#![allow(unsafe_code)]

use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use procraft_core::CanvasSession;
use winit::event::WindowEvent;

use crate::app::{App, Loop};
use crate::host::Host;
use crate::ohos_input::Input as TabletInput;
use crate::ohos_screen::OhosScreen;
use crate::ohos_window::NativeWindow;


/// The app's log, to the system's (hilog, where `hdc shell hilog` reads it): a tablet has no
/// console, and a start that fails leaves nothing else to read.
struct Hilog;

impl log::Log for Hilog {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        // the GL driver's own probing of what it supports is noise (thousands of lines a second),
        // and says nothing to act on: the app's own is what is wanted
        if metadata.target().starts_with("wgpu_hal::gles") || metadata.target().starts_with("wgpu_core::") {
            return false;
        }
        metadata.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        // the system's own levels (`LogLevel` in hilog/log.h): 3 debug, 4 info, 5 warn, 6 error,
        // 7 fatal (a bigger number is more severe)
        let level = match record.level() {
            log::Level::Error => 6,
            log::Level::Warn => 5,
            log::Level::Info => 4,
            log::Level::Debug => 3,
            log::Level::Trace => 3,
        };
        hilog(level, &format!("{}: {}", record.target(), record.args()));
    }

    fn flush(&self) {}
}

/// Writes one line to the system's log (`level`: 3 debug, 4 info, 5 warn, 6 error, as hilog's).
pub fn hilog(level: i32, text: &str) {
    let Ok(text) = std::ffi::CString::new(text.replace('\0', " ")) else { return };
    let Ok(tag) = std::ffi::CString::new("ProCraft") else { return };
    let Ok(format) = std::ffi::CString::new("%{public}s") else { return };
    // SAFETY: the system's own logging call, with live strings (its own order: the log's kind, its
    // level, the domain, the tag, then the format and what it fills)
    unsafe {
        OhLogPrint(LOG_APP, level as u32, 0x3200, tag.as_ptr(), format.as_ptr(), text.as_ptr());
    }
}

/// The app's own log's kind (`LOG_APP`, the system's own).
const LOG_APP: u32 = 0;

#[link(name = "hilog_ndk.z")]
unsafe extern "C" {
    #[link_name = "OH_LOG_Print"]
    fn OhLogPrint(
        kind: u32,
        level: u32,
        domain: u32,
        tag: *const std::ffi::c_char,
        format: *const std::ffi::c_char,
        ...
    ) -> std::ffi::c_int;
}

/// Sets the app's log: the system's, and its own folder's. A panic goes there too: a tablet's app
/// that stops has nowhere else to say why, and the system's own report is not readable from here.
fn init_log() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = log::set_logger(&Hilog);
        log::set_max_level(log::LevelFilter::Info);
        let before = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic| {
            // (the app's own thread's panics are caught where a driver's are, which says so)
            let payload = panic.payload();
            let text = if let Some(s) = payload.downcast_ref::<&str>() {
                (*s).to_owned()
            } else if let Some(s) = payload.downcast_ref::<String>() {
                s.clone()
            } else {
                "a panic".to_owned()
            };
            log::error!("panicked at {}: {text}", panic.location().map_or("?".to_owned(), |l| l.to_string()));
            before(panic);
        }));
        log::info!("the log is up");
    });
}

/// The loop as the app sees it: a flag and a condition, woken by the shell.
pub struct TabletLoop {
    /// The app is to stop.
    stopped: Arc<AtomicBool>,
    /// A frame is wanted.
    wanted: Arc<(Mutex<bool>, Condvar)>,
}

impl TabletLoop {
    fn new() -> TabletLoop {
        TabletLoop { stopped: Arc::new(AtomicBool::new(false)), wanted: Arc::new((Mutex::new(false), Condvar::new())) }
    }

    /// A frame is wanted now (a touch came, a key did, or the interface asked for one).
    pub fn wake(&self) {
        let (wanted, wait) = &*self.wanted;
        *wanted.lock().unwrap_or_else(|e| e.into_inner()) = true;
        wait.notify_all();
    }

    fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Relaxed)
    }

    /// The loop is to stop (its surface went): it is woken so it can keep what is open as it goes.
    fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        self.wake();
    }
}

impl Loop for TabletLoop {
    fn exit(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        self.wake();
    }

    fn wait(&self) {
        let (wanted, wait) = &*self.wanted;
        let mut wanted = wanted.lock().unwrap_or_else(|e| e.into_inner());
        while !*wanted && !self.stopped() {
            wanted = wait.wait(wanted).unwrap_or_else(|e| e.into_inner());
        }
        *wanted = false;
    }

    fn wait_until(&self, at: Instant) {
        let (wanted, wait) = &*self.wanted;
        let mut wanted = wanted.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        if at > now {
            let (guard, _) = wait.wait_timeout(wanted, at - now).unwrap_or_else(|e| e.into_inner());
            wanted = guard;
        }
        *wanted = false;
    }
}

/// The window as the app sees it: the XComponent's surface, and the input the shell gathered.
pub struct TabletHost {
    pub screen: OhosScreen,
    pub input: TabletInput,
    /// Wakes the app's thread (the interface asking for another frame).
    pub wake: Arc<TabletLoop>,
}

impl Host for TabletHost {
    fn size(&self) -> (u32, u32) {
        self.screen.size()
    }

    fn scale(&self) -> f32 {
        self.screen.pixels_per_point()
    }

    fn ctx(&self) -> &egui::Context {
        &self.screen.ctx
    }

    fn gpu(&self) -> &procraft_gpu::Gpu {
        &self.screen.gpu
    }

    fn request_redraw(&self) {
        self.wake.wake();
    }

    fn set_title(&self, _title: &str) {
        // the tablet has no title bar of its own: the app's name is on its icon
    }

    fn window_icon(&self) -> Option<winit::window::Icon> {
        None
    }

    fn frame_gap(&self) -> Duration {
        // a tablet's display refreshes at 60, 90 or 120 Hz; a frame is paced at 60 at least
        Duration::from_millis(16)
    }

    fn minimised(&self) -> bool {
        let (width, height) = self.screen.size();
        width == 0 || height == 0
    }

    fn take_input(&mut self) -> egui::RawInput {
        let (_, events, modifiers, focused) = self.input.take_events();
        self.screen.take_input(events, modifiers, focused)
    }

    fn draw(&mut self, session: Option<&CanvasSession>, started: Instant, output: egui::FullOutput) -> Option<Duration> {
        self.screen.draw(session, started, output)
    }

    fn set_pixels_per_point(&mut self, pixels_per_point: f32) {
        self.screen.set_pixels_per_point(pixels_per_point);
    }

    fn size_interface(&self, plugins: &procraft_core::Plugins, session: Option<&CanvasSession>) {
        self.screen.size_interface(plugins, session);
    }

    fn over_interface(&self, p: (f32, f32), canvas: egui::Rect) -> bool {
        self.screen.over_interface(p, canvas)
    }

    fn draw_pad(&mut self, pad: &CanvasSession, size: (u32, u32), started: Instant) -> egui::TextureId {
        self.screen.draw_pad(pad, size, started)
    }

    fn drop_pad(&mut self) {
        self.screen.drop_pad();
    }

    fn canvases_made(&mut self) -> Option<bool> {
        self.screen.canvases_made()
    }

    fn canvases_waited(&self) -> Option<Duration> {
        self.screen.canvases_waited()
    }

    fn gl_busy(&self) -> bool {
        false
    }

    fn canvases_later(&self) -> bool {
        false
    }

    fn tried(&self) -> (usize, bool) {
        (0, true)
    }

    fn set_visible(&self, _shown: bool) {
        // nothing to hide: the surface is the XComponent's, which the shell shows
    }
}

/// What the shell tells the app's own thread: everything the system gave it.
enum Said {
    Touch { id: u64, phase: u32, x: f64, y: f64, pressure: f64, tilt: f64, orientation: f64, pen: bool, eraser: bool },
    Hover { id: u64, x: f64, y: f64, tilt: f64, orientation: f64, eraser: bool },
    HoverOut { id: u64 },
    Key { code: u32, text: u32, pressed: bool, repeat: bool },
    Text(String),
    Resized { width: u32, height: u32 },
    Scale { pixels_per_point: f32 },
    Focused(bool),
    Dropped(String),
    Wake,
    Stop,
}

/// The app's own thread's mailbox, and the way to wake it.
static SAID: Mutex<Option<std::sync::mpsc::Sender<Said>>> = Mutex::new(None);
/// The app's own thread, while one runs: a surface that goes stops it, and the next start waits
/// for it to have stopped (the XComponent may make a new surface at any time: the page turned, the
/// window resized, the app brought back).
static RUNNING: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);
/// Wakes the app's thread from the shell's.
static WAKE: Mutex<Option<Arc<TabletLoop>>> = Mutex::new(None);

/// Tells the app's thread what the system gave, waking it.
fn tell(said: Said) {
    if let Some(sender) = SAID.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        let _ = sender.send(said);
    }
    if let Some(loop_) = WAKE.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        loop_.wake();
    }
}

/// The window's surface is ready (the XComponent made one): the app starts, drawing in it on a
/// thread of its own.
///
/// # Safety
/// `window` is a live `OHNativeWindow` from the shell's XComponent, kept alive until
/// `procraft_surface_destroyed` says otherwise.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn procraft_own_folder(bytes: *const u8, length: usize) {
    if bytes.is_null() || length == 0 {
        return;
    }
    let slice = std::slice::from_raw_parts(bytes, length);
    if let Ok(dir) = std::str::from_utf8(slice) {
        crate::files::set_own_folder(dir);
        log::info!("the app's own files go in {dir}");
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn procraft_start(window: *mut c_void, width: u32, height: u32, pixels_per_point: f32) {
    if window.is_null() {
        hilog(6, "procraft_start: no window to draw in");
        return;
    }
    // the log, to the app's own folder (and the system's, for a device with no console)
    init_log();
    // (whether the system's own fonts can be read here at all: the app draws every script from
    // them, and an app's sandbox may not be let near `/system`)
    for font in ["/system/fonts/HarmonyOS_Sans_SC.ttf", "/system/fonts/NotoSansCJK-Regular.ttc", "/system/fonts/HarmonyOS_Sans.ttf"] {
        match std::fs::read(font) {
            Ok(bytes) => log::warn!("font {font}: readable ({} bytes)", bytes.len()),
            Err(e) => log::warn!("font {font}: {e}"),
        }
    }
    log::info!("ProCraft starts: {width}x{height} pixels, {pixels_per_point} pixels per point");
    // a surface that went left its app stopping: waited for here (this is the app's own thread,
    // not the shell's), so two never draw at once
    join_stopped();
    let loop_ = Arc::new(TabletLoop::new());
    let wake = {
        let loop_ = Arc::clone(&loop_);
        Arc::new(move || loop_.wake())
    };
    let order: Vec<usize> = (0..procraft_gpu::Gpu::backend_count().max(1)).collect();
    let window = NativeWindow(window);
    let made = OhosScreen::new(
        &window,
        (width, height),
        pixels_per_point,
        &order,
        0,
        |backend| log::info!("trying graphics backend {backend}"),
        {
            let wake = Arc::clone(&wake);
            move || wake()
        },
    );
    let (screen, _) = match made {
        Ok(made) => made,
        Err(e) => {
            // (said through the queue, never shown from here: this is the shell's own thread,
            // which is the only thread that could show it, so showing it here would wait on
            // itself and the message would never come)
            log::error!("the window's surface couldn't be set up: {e}");
            crate::ohos_platform::say_unwaited("ProCraft", &format!("{e}"), true);
            return;
        }
    };
    let mut app = App::<TabletHost, TabletLoop>::new({
        let wake = Arc::clone(&wake);
        move || wake()
    });
    app.resumed(TabletHost { screen, input: TabletInput::new(pixels_per_point), wake: Arc::clone(&loop_) });
    let (sender, said) = std::sync::mpsc::channel();
    *SAID.lock().unwrap_or_else(|e| e.into_inner()) = Some(sender);
    *WAKE.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::clone(&loop_));
    match std::thread::Builder::new().name("procraft-tablet".to_owned()).spawn(move || {
        run_loop(app, loop_, said);
    }) {
        Ok(thread) => *RUNNING.lock().unwrap_or_else(|e| e.into_inner()) = Some(thread),
        Err(e) => log::error!("the app's thread: {e}"),
    }
}

/// Tells the app's own thread to stop, without waiting for it.
///
/// This is called from the shell's own thread (the surface went), which is the thread the app's
/// asks are answered on: waiting there for the app to finish would wait on the answer that thread
/// alone can give, and hang the window for as long as the ask's own timeout. So the stopping is
/// only asked for here; [`join_stopped`] waits, and is called from the app's next start instead.
fn stop_app() {
    if let Some(loop_) = WAKE.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        // (the loop keeps what is open as it goes)
        loop_.stop();
    }
}

/// Waits for a stopping app's thread to have gone: called before another starts, which is the
/// app's own thread (`procraft_start`), never the shell's.
fn join_stopped() {
    let running = RUNNING.lock().unwrap_or_else(|e| e.into_inner()).take();
    let Some(running) = running else { return };
    if running.join().is_err() {
        log::error!("the app's thread stopped with a panic");
    }
    *SAID.lock().unwrap_or_else(|e| e.into_inner()) = None;
    *WAKE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// The app's own loop: what the shell says, a frame when one is wanted, and the touches' waits.
///
/// The desktop's loop is winit's, which asks for a redraw when the app wants one and otherwise
/// waits; this one is the same, with the shell's own wakeups. A frame is drawn when the app asked
/// for one (`App::again`, set by the last draw), or when nothing else happened this pass and the
/// canvas wants one by itself (`CanvasSession::wants_frame`) — the same two the desktop's
/// `about_to_wait` draws for. Frames are paced to the display: without that a canvas that wants a
/// frame every time it is drawn draws as fast as the CPU goes.
///
/// The touches have waits of their own — a hold, a repeat, the time a tap has — which run out
/// between the events: the desktop's loop runs them in `about_to_wait`, and the canvas needs them
/// (two fingers tapped at once undo, three redo, a held pen picks a colour). So they run before
/// each frame, and the loop sleeps no longer than the next one is due.
fn run_loop(mut app: App<TabletHost, TabletLoop>, loop_: Arc<TabletLoop>, said: std::sync::mpsc::Receiver<Said>) {
    // when a frame was last drawn (the pacing goes by it: `App::last_frame` is set as a frame
    // begins, so it is "now" by the time the wait is decided, and would pace nothing)
    let mut last_drawn = Instant::now() - Duration::from_secs(1);
    loop {
        // what came while the last frame was drawn
        while let Ok(said) = said.try_recv() {
            take(&mut app, said);
        }
        if loop_.stopped() {
            break;
        }
        pump_events(&mut app, &loop_);
        // what the files written on their own threads had to say (a save that failed, an export
        // done): the desktop's loop says it each pass, and nothing else ever would
        crate::files::tell_what_was_told();
        // the graphics device stopped (a driver reset, the system taking the GPU away): nothing it
        // made can be drawn again. The desktop starts itself again on another backend, which this
        // window can't do (its surface is the shell's): it says so and keeps the artwork, so what
        // is open is saved and can be opened again when the app is.
        if lost_told(&app) {
            log::error!("the graphics device stopped");
            if let Some(session) = app.session.clone() {
                let _ = session.save(app.artwork.clone().unwrap_or_default());
                session.stop_autosave();
            }
            crate::platform::say(
                "ProCraft",
                "The graphics device stopped. ProCraft keeps your artwork: open it again from the gallery.",
                true,
            );
            app.session = None;
            return;
        }
        // the canvases' pipelines are made (they take seconds): the gallery stops saying it is
        // getting ready, and the window draws the canvas it opens. The desktop's `about_to_wait`
        // does the same; without it the gallery waits on them for good, asking for a frame every
        // time and never getting one that shows.
        if let Some(fine) = app.host.as_mut().and_then(|h| h.canvases_made()) {
            app.canvases_made(fine, &loop_);
            loop_.wake();
        }
        // whatever the touches' waits have made of it is in before the frame draws it
        let waiting = touch_waits(&mut app);
        let now = Instant::now();
        let gap = app.host.as_ref().map_or(Duration::from_millis(16), crate::host::Host::frame_gap);
        // a frame is due: the one the last asked for, or (nothing else drawing) the canvas's own
        let asked = app.again.take();
        // what the touches asked the app to do (undo, a fill, the view moved) is drawn for, and
        // so is the canvas wanting a frame of its own: the desktop's `about_to_wait` asks for a
        // redraw for the first of these, and the canvas's own want is the other
        let commands_wait = !app.input.commands.is_empty();
        let canvas_wants = (commands_wait || app.session.as_ref().is_some_and(|s| s.wants_frame()))
            && app.host.as_ref().is_some_and(|h| !h.minimised());
        let frame_due = asked.or(canvas_wants.then_some(now)).map(|due| due.max(last_drawn + gap));
        if frame_due.is_none_or(|due| due <= now) {
            frame(&mut app);
            last_drawn = Instant::now();
        }
        // when the next frame is due: what this draw asked for, or the canvas's own want (paced:
        // a frame that couldn't be drawn yet keeps its own time, or nothing would wake for it)
        let next_frame = app
            .again
            .take()
            .map(|due| due.max(last_drawn + gap))
            .or_else(|| canvas_wants.then(|| last_drawn + gap))
            .or(frame_due);
        // a wait that has run out and changed nothing (a repeat due while nothing is held) would
        // have this loop pass it by again and again: one already past is not waited for
        let waiting = waiting.filter(|at| *at > Instant::now());
        // wait for the next wake, or for the frame or the touch that is due
        match match (next_frame, waiting) {
            (Some(frame), Some(when)) => Some(frame.min(when)),
            (frame, when) => frame.or(when),
        } {
            Some(at) if at <= Instant::now() => {}
            Some(at) => loop_.wait_until(at),
            None => loop_.wait(),
        }
    }
}

/// Whether the graphics device has stopped, said once (the loop asks each pass).
fn lost_told(app: &App<TabletHost, TabletLoop>) -> bool {
    static TOLD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let lost = app.host.as_ref().is_some_and(|h| h.gpu().is_lost());
    if !lost {
        return false;
    }
    !TOLD.swap(true, std::sync::atomic::Ordering::Relaxed)
}

/// Runs the touches' waits that have run out, and says when the next one is due (none when the
/// touches wait for nothing). What came of them is drawn by the frame after it: the canvas's own
/// input state machine keeps the taps (`app.input`), not the platform's touches (`host.input`).
fn touch_waits(app: &mut App<TabletHost, TabletLoop>) -> Option<Instant> {
    let session = app.session.clone()?;
    app.host.as_ref()?;
    app.input.tick(&session);
    app.input.next_deadline(&session)
}

/// Takes one thing the shell said.
fn take(app: &mut App<TabletHost, TabletLoop>, said: Said) {
    use winit::event::TouchPhase;
    match said {
        Said::Stop => {
            app.leave_artwork();
            crate::files::finish_writing();
        }
        Said::Touch { id, phase, x, y, pressure, tilt, orientation, pen, eraser } => {
            let phase = match phase {
                0 => TouchPhase::Started,
                1 => TouchPhase::Moved,
                2 => TouchPhase::Ended,
                _ => TouchPhase::Cancelled,
            };
            if let Some(host) = app.host.as_mut() {
                host.input.touch(id, phase, x, y, pressure, tilt, orientation, pen, eraser);
            }
        }
        Said::Hover { id, x, y, tilt, orientation, eraser } => {
            if let Some(host) = app.host.as_mut() {
                host.input.hover(id, x, y, tilt, orientation, eraser);
            }
        }
        Said::HoverOut { id } => {
            if let Some(host) = app.host.as_mut() {
                host.input.hover_out(id);
            }
        }
        Said::Key { code, text, pressed, repeat } => {
            let (named, key_code, character) = key_of(code, text);
            if let Some(host) = app.host.as_mut() {
                host.input.key(named, key_code, character, pressed, repeat);
            }
        }
        Said::Text(text) => {
            if let Some(host) = app.host.as_mut() {
                host.input.text(&text);
            }
        }
        Said::Scale { pixels_per_point } => {
            if let Some(host) = app.host.as_mut() {
                host.input.density = pixels_per_point;
                host.screen.set_pixels_per_point(pixels_per_point);
            }
            // the canvas draws in window pixels per point too (brushes, touches, what shows
            // where): the desktop takes the window's change the same way
            if let Some(session) = &app.session {
                session.set_display_scale(pixels_per_point);
                let scale = app.host.as_ref().map_or(pixels_per_point, |h| h.scale() * h.ctx().zoom_factor());
                app.ui.place_canvas(session, scale);
            }
        }
        Said::Resized { width, height } => {
                if let Some(host) = app.host.as_mut() {
                host.screen.resize(width, height);
                host.input.resized(width, height);
            }
            if let Some(session) = &app.session {
                session.resize_window(width, height);
            }
        }
        Said::Focused(focused) => {
            if let Some(host) = app.host.as_mut() {
                host.input.focused(focused);
            }
        }
        Said::Dropped(path) => {
            if let Some(host) = app.host.as_mut() {
                host.input.dropped_file(std::path::PathBuf::from(path));
            }
        }
        Said::Wake => {}
    }
}

/// The system's key code, and what it types, as the app's own (`winit`'s).
///
/// The two numberings have nothing in common: the system's (`oh_key_code.h`) keeps its named keys
/// in single digits and its letters, digits and punctuation as runs from `KEYCODE_0` and
/// `KEYCODE_A` on, while `winit`'s follow a PC keyboard's scan codes. So each is named here, and
/// a key with no name of the app's own (a function key, a media key) is left as the character it
/// typed, if any.
fn key_of(code: u32, text: u32) -> (Option<winit::keyboard::NamedKey>, Option<winit::keyboard::KeyCode>, Option<char>) {
    use winit::keyboard::{KeyCode, NamedKey};
    /// The keys the app has a name for, and the system's own name for each: `oh_key_code.h`'s.
    /// (The punctuation has no name of the app's — only a code — so it is a table of its own.)
    const NAMED: &[(u32, NamedKey, KeyCode)] = &[
        // the back key: the desktop closes what is open with Escape, and so does this
        (2, NamedKey::Escape, KeyCode::Escape), // KEYCODE_BACK
        (2012, NamedKey::ArrowUp, KeyCode::ArrowUp), // KEYCODE_DPAD_UP
        (2013, NamedKey::ArrowDown, KeyCode::ArrowDown), // KEYCODE_DPAD_DOWN
        (2014, NamedKey::ArrowLeft, KeyCode::ArrowLeft), // KEYCODE_DPAD_LEFT
        (2015, NamedKey::ArrowRight, KeyCode::ArrowRight), // KEYCODE_DPAD_RIGHT
        (2045, NamedKey::Alt, KeyCode::AltLeft), // KEYCODE_ALT_LEFT
        (2046, NamedKey::Alt, KeyCode::AltRight), // KEYCODE_ALT_RIGHT
        (2047, NamedKey::Shift, KeyCode::ShiftLeft), // KEYCODE_SHIFT_LEFT
        (2048, NamedKey::Shift, KeyCode::ShiftRight), // KEYCODE_SHIFT_RIGHT
        (2049, NamedKey::Tab, KeyCode::Tab), // KEYCODE_TAB
        (2050, NamedKey::Space, KeyCode::Space), // KEYCODE_SPACE
        (2054, NamedKey::Enter, KeyCode::Enter), // KEYCODE_ENTER
        (2055, NamedKey::Backspace, KeyCode::Backspace), // KEYCODE_DEL (the one that goes back)
        (2068, NamedKey::PageUp, KeyCode::PageUp), // KEYCODE_PAGE_UP
        (2069, NamedKey::PageDown, KeyCode::PageDown), // KEYCODE_PAGE_DOWN
        (2070, NamedKey::Escape, KeyCode::Escape), // KEYCODE_ESCAPE
        (2071, NamedKey::Delete, KeyCode::Delete), // KEYCODE_FORWARD_DEL
        (2072, NamedKey::Control, KeyCode::ControlLeft), // KEYCODE_CTRL_LEFT
        (2073, NamedKey::Control, KeyCode::ControlRight), // KEYCODE_CTRL_RIGHT
        (2081, NamedKey::Home, KeyCode::Home), // KEYCODE_MOVE_HOME
        (2082, NamedKey::End, KeyCode::End), // KEYCODE_MOVE_END
    ];
    /// The punctuation: it has a code of the app's, but no name.
    const PUNCTUATION: &[(u32, KeyCode, char)] = &[
        (2043, KeyCode::Comma, ','),        // KEYCODE_COMMA
        (2044, KeyCode::Period, '.'),       // KEYCODE_PERIOD
        (2056, KeyCode::Backquote, '`'),    // KEYCODE_GRAVE
        (2057, KeyCode::Minus, '-'),        // KEYCODE_MINUS
        (2058, KeyCode::Equal, '='),        // KEYCODE_EQUALS
        (2059, KeyCode::BracketLeft, '['),  // KEYCODE_LEFT_BRACKET
        (2060, KeyCode::BracketRight, ']'), // KEYCODE_RIGHT_BRACKET
        (2061, KeyCode::Backslash, '\\'),   // KEYCODE_BACKSLASH
        (2062, KeyCode::Semicolon, ';'),    // KEYCODE_SEMICOLON
        (2063, KeyCode::Quote, '\''),       // KEYCODE_APOSTROPHE
        (2064, KeyCode::Slash, '/'),        // KEYCODE_SLASH
    ];
    let (named, key_code) = match NAMED.iter().find(|(at, ..)| *at == code) {
        Some((_, named, key_code)) => (Some(*named), Some(*key_code)),
        // the digits, `0` on, and the letters, `A` on (a run each), and the punctuation
        None => match PUNCTUATION.iter().find(|(at, ..)| *at == code) {
            Some((_, key_code, _)) => (None, Some(*key_code)),
            None => match code {
                2000..=2009 => (None, Some(digit_key(code - 2000))),
                2017..=2042 => (None, Some(letter_key(code - 2017))),
                _ => (None, None),
            },
        },
    };
    // what it types: what the system gave, or failing that the letter, digit or punctuation the
    // key is (the system's `text` is empty for the keys a soft keyboard would have typed)
    let typed = (text != 0)
        .then(|| char::from_u32(text))
        .flatten()
        .or_else(|| match PUNCTUATION.iter().find(|(at, ..)| *at == code) {
            Some((.., typed)) => Some(*typed),
            None => match code {
                2000..=2009 => char::from_digit(code - 2000, 10),
                2017..=2042 => char::from_u32(u32::from(b'a') + (code - 2017)),
                _ => None,
            },
        });
    (named, key_code, typed)
}

/// The system's `KEYCODE_0` run, as the app's own digits (`0` is at 0).
fn digit_key(offset: u32) -> winit::keyboard::KeyCode {
    use winit::keyboard::KeyCode;
    const DIGITS: [KeyCode; 10] = [
        KeyCode::Digit0,
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
    ];
    DIGITS.get(offset as usize).copied().unwrap_or(KeyCode::Digit0)
}

/// The system's `KEYCODE_A` run, as the app's own letters (`A` is at 0).
fn letter_key(offset: u32) -> winit::keyboard::KeyCode {
    use winit::keyboard::KeyCode;
    const LETTERS: [KeyCode; 26] = [
        KeyCode::KeyA,
        KeyCode::KeyB,
        KeyCode::KeyC,
        KeyCode::KeyD,
        KeyCode::KeyE,
        KeyCode::KeyF,
        KeyCode::KeyG,
        KeyCode::KeyH,
        KeyCode::KeyI,
        KeyCode::KeyJ,
        KeyCode::KeyK,
        KeyCode::KeyL,
        KeyCode::KeyM,
        KeyCode::KeyN,
        KeyCode::KeyO,
        KeyCode::KeyP,
        KeyCode::KeyQ,
        KeyCode::KeyR,
        KeyCode::KeyS,
        KeyCode::KeyT,
        KeyCode::KeyU,
        KeyCode::KeyV,
        KeyCode::KeyW,
        KeyCode::KeyX,
        KeyCode::KeyY,
        KeyCode::KeyZ,
    ];
    LETTERS.get(offset as usize).copied().unwrap_or(KeyCode::KeyA)
}

/// Takes what the shell gathered since the last frame: its touches' and keys' own events, and the
/// interface's (egui's).
fn pump_events(app: &mut App<TabletHost, TabletLoop>, app_loop: &TabletLoop) {
    let keys = match app.host.as_mut() {
        Some(host) => host.input.take_keys(),
        None => return,
    };
    let window_events = match app.host.as_mut() {
        Some(host) => std::mem::take(&mut host.input.window_events),
        None => return,
    };
    let session = app.session.clone();
    if let Some(session) = &session {
        for (code, pressed, repeat) in keys {
            let consumed = app.ui.popover_open;
            app.input.on_key(session, code, pressed, repeat, consumed);
        }
    }
    for event in window_events {
        match event {
            WindowEvent::CloseRequested => {
                app_loop.exit();
                return;
            }
            WindowEvent::Resized(size) => {
                if let Some(host) = app.host.as_mut() {
                    host.screen.resize(size.width, size.height);
                }
            }
            event => {
                let Some(session) = &session else { continue };
                let Some(host) = app.host.as_mut() else { return };
                let through = app.ui.popover_open && app.ui.panel_lets_through;
                let blocked = (app.ui.popover_open && !through)
                    || crate::files::exporting_since().is_some()
                    || session.pour_finishing();
                session.set_touch_panel_open(through);
                let canvas = app.ui.canvas_rect;
                let over = |p: (f32, f32)| blocked || host.over_interface(p, canvas);
                let consumed = host.ctx().egui_wants_keyboard_input();
                if app.input.on_event(session, &event, consumed, over) {
                    host.request_redraw();
                }
            }
        }
    }
}

/// One frame: the interface, and the canvas under it when one is open. Whatever came in on a
/// thread of its own is taken up first, with a canvas open or not: an artwork being read (the
/// gallery asked for it), a picture, files brought in. The desktop's loop does the same before it
/// chooses which of the two to draw (see `App::frame`), and without it an artwork opened from the
/// gallery never arrives (no canvas is open yet to draw it with).
fn frame(app: &mut App<TabletHost, TabletLoop>) {
    app.take_dropped();
    app.imported();
    app.artwork_read();
    app.photo_read();
    if app.session.is_some() {
        app.frame();
    } else {
        app.gallery_frame();
    }
}

/// The window's surface is gone (the XComponent went away, or the app is ending).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn procraft_surface_destroyed() {
    // the app keeps what is open and its thread stops: another surface starts it again (the page
    // turned, the window resized, the app brought back), which waits for this one to have gone
    tell(Said::Stop);
    stop_app();
}

/// A frame is to be drawn now (a touch came, a key did, the interface asked).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn procraft_wake() {
    tell(Said::Wake);
}

/// A touch came: `phase` is 0 (down), 1 (move), 2 (up) or 3 (cancel).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn procraft_touch(
    id: u64,
    phase: u32,
    x: f64,
    y: f64,
    pressure: f64,
    tilt: f64,
    orientation: f64,
    pen: bool,
    eraser: bool,
) {
    tell(Said::Touch { id, phase, x, y, pressure, tilt, orientation, pen, eraser });
}

/// A pen is near but not touching.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn procraft_hover(id: u64, x: f64, y: f64, tilt: f64, orientation: f64, eraser: bool) {
    tell(Said::Hover { id, x, y, tilt, orientation, eraser });
}

/// A pen left the tablet's reach.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn procraft_hover_out(id: u64) {
    tell(Said::HoverOut { id });
}

/// A key went down or up: `code` is the system's key code, `text` what it types (0 for none).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn procraft_key(code: u32, text: u32, pressed: bool, repeat: bool) {
    tell(Said::Key { code, text, pressed, repeat });
}

/// What a soft keyboard typed (UTF-8).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn procraft_text(bytes: *const u8, length: usize) {
    if bytes.is_null() || length == 0 {
        return;
    }
    let slice = std::slice::from_raw_parts(bytes, length);
    if let Ok(text) = std::str::from_utf8(slice) {
        tell(Said::Text(text.to_owned()));
    }
}

/// The window's size changed (the panel turned, or its window did).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn procraft_resized(width: u32, height: u32) {
    tell(Said::Resized { width, height });
}

/// The page's density (its points to pixels).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn procraft_scale(pixels_per_point: f32) {
    tell(Said::Scale { pixels_per_point });
}

/// The app went to the background, or came back.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn procraft_focused(focused: bool) {
    tell(Said::Focused(focused));
}

/// A file was opened with the app (a picture, an artwork), its path as UTF-8.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn procraft_dropped_file(bytes: *const u8, length: usize) {
    if bytes.is_null() || length == 0 {
        return;
    }
    let slice = std::slice::from_raw_parts(bytes, length);
    if let Ok(path) = std::str::from_utf8(slice) {
        tell(Said::Dropped(path.to_owned()));
    }
}

/// What the interface asked the shell for, waiting to be taken (`ohos_platform`): a file picked,
/// a message shown, the clipboard read.
#[unsafe(no_mangle)]
pub extern "C" fn procraft_asks() -> *mut std::ffi::c_char {
    let taken = crate::ohos_platform::take();
    let mut json = String::from("[");
    for (at, (id, kind, details)) in taken.iter().enumerate() {
        if at > 0 {
            json.push(',');
        }
        json.push_str(&format!("{{\"id\":{id},\"kind\":\"{kind}\",\"details\":{details}}}"));
    }
    json.push(']');
    match std::ffi::CString::new(json) {
        Ok(text) => text.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Lets go of what `procraft_asks` gave.
///
/// # Safety
/// `text` is what `procraft_asks` returned, given back once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn procraft_asks_free(text: *mut std::ffi::c_char) {
    if !text.is_null() {
        drop(std::ffi::CString::from_raw(text));
    }
}

/// The shell's answer to ask `id`: the paths picked (each on its own line), the clipboard's
/// contents, or nothing.
///
/// # Safety
/// `paths` and `clipboard` are UTF-8 the shell owns, read only while this runs.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn procraft_answer(
    id: u64,
    paths: *const std::ffi::c_char,
    clipboard: *const std::ffi::c_char,
    done: bool,
) {
    let text = |pointer: *const std::ffi::c_char| -> Option<String> {
        if pointer.is_null() {
            return None;
        }
        std::ffi::CStr::from_ptr(pointer).to_str().ok().map(str::to_owned)
    };
    let paths = text(paths).map(|p| p.lines().map(str::to_owned).filter(|l| !l.is_empty()).collect()).unwrap_or_default();
    let clipboard = text(clipboard).and_then(|c| crate::ohos_platform::clipboard_of(&c));
    crate::ohos_platform::answer(id, crate::ohos_platform::Answered { paths, clipboard, done });
}
