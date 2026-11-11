//! The tablet's touches, its pen and its keys, as the events the interface already knows: the
//! desktop's `Input` takes `winit::event::WindowEvent`s (its touches carry a pen's pressure and
//! lean), and egui takes its own (a pointer's moves, presses and scrolls, keys and text). Both
//! come from what the XComponent hands over here: its `OH_NativeXComponent_TouchEvent`s, its
//! hover and its keys.
//!
//! This is the tablet's `pen.rs` and half of its `main.rs`: where the desktop's events come from
//! winit, the tablet's come from the system's own, and are made into the same shapes, so that
//! everything above — the input state machine, the gestures, the interface — is the same code.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use winit::dpi::PhysicalPosition;
use winit::event::{DeviceId, Force, MouseScrollDelta, Touch, TouchPhase, WindowEvent};
use winit::keyboard::{KeyCode, ModifiersState, NamedKey};


/// egui's own key for one the system gave: its name, as egui names keys ("A", "Space", "ArrowLeft").
fn egui_key(named: Option<NamedKey>, character: Option<char>) -> Option<egui::Key> {
    if let Some(c) = character {
        // a letter or a digit, as egui names them
        let name = c.to_ascii_uppercase().to_string();
        if let Some(key) = egui::Key::from_name(&name) {
            return Some(key);
        }
    }
    let name = match named? {
        NamedKey::Space => "Space",
        NamedKey::Enter => "Enter",
        NamedKey::Tab => "Tab",
        NamedKey::Backspace => "Backspace",
        NamedKey::Delete => "Delete",
        NamedKey::Escape => "Escape",
        NamedKey::ArrowLeft => "ArrowLeft",
        NamedKey::ArrowRight => "ArrowRight",
        NamedKey::ArrowUp => "ArrowUp",
        NamedKey::ArrowDown => "ArrowDown",
        NamedKey::Home => "Home",
        NamedKey::End => "End",
        NamedKey::PageUp => "PageUp",
        NamedKey::PageDown => "PageDown",
        NamedKey::Shift => "Shift",
        NamedKey::Control => "Control",
        NamedKey::Alt => "Alt",
        NamedKey::Meta => "Meta",
        _ => return None,
    };
    egui::Key::from_name(name)
}

/// A modifier key went down or up: its flag, as egui wants it, and the window's state of what is
/// held, both set, and whether that was news (nothing is said of a key that was held already, or
/// of the release of one that was not).
fn held(flag: &mut bool, state: &mut ModifiersState, bit: ModifiersState, down: bool) -> bool {
    if *flag == down {
        return false;
    }
    *flag = down;
    if down {
        *state |= bit;
    } else {
        // (`ModifiersState::remove` changes what it is given, returning nothing to assign)
        state.remove(bit);
    }
    true
}

/// The tablet's pointers, as they were last heard from: what each is (a pen or a finger), where
/// it is, its pressure and its lean, and whether it is down.
#[derive(Clone, Debug, Default)]
struct Pointer {
    /// Whether it is a pen (the system says: a stylus, or a finger).
    pen: bool,
    /// Where it is, in points.
    at: (f64, f64),
    /// 0..1, as the system gives it.
    pressure: f64,
    /// Radians from upright, and the direction it leans.
    tilt: f64,
    orientation: f64,
    /// Whether it is down.
    down: bool,
    /// Whether it is an eraser's end.
    eraser: bool,
    /// When it came down (for a tap's timing).
    began: Option<Instant>,
}

/// What the platform gathered since the last frame, and what it says the app should know: the
/// events (both as the interface's `winit` ones and as egui's), the modifiers held, whether the
/// window has focus, and whether a frame is wanted.
#[derive(Default)]
pub struct Input {
    /// The events for the interface's own input (`crate::input::Input`), as winit's.
    pub window_events: Vec<WindowEvent>,
    /// The events for egui: a pointer's moves and presses, keys, text, scrolls.
    pub egui_events: Vec<egui::Event>,
    /// The keys and buttons held, as egui wants them each frame.
    pub modifiers: egui::Modifiers,
    /// The modifiers as winit's input state wants them.
    pub winit_modifiers: ModifiersState,
    /// Whether the window has focus.
    pub focused: bool,
    /// The pointers down or hovering, by the system's id.
    pointers: HashMap<u64, Pointer>,
    /// The keys that came, for the interface's own two (Space, R).
    pub keys: Vec<(KeyCode, bool, bool)>,
    /// Pixels per point, as the page's own density (egui works in points).
    pub density: f32,
    /// A frame is wanted (a touch came, a key did, or the interface asked).
    pub wants_frame: bool,
    /// Pixels per point, as the page's own density.
    /// The cursor's icon as the interface last asked for it (a pen's brush outline is drawn by
    /// the canvas itself; this is for a mouse's arrow).
    pub cursor: Option<egui::CursorIcon>,
}

static CURSOR: Mutex<Option<egui::CursorIcon>> = Mutex::new(None);

/// The cursor the interface last asked for.
pub fn set_cursor(icon: egui::CursorIcon) {
    *CURSOR.lock().unwrap_or_else(|e| e.into_inner()) = Some(icon);
}

impl Input {
    pub fn new(pixels_per_point: f32) -> Input {
        Input { focused: true, density: pixels_per_point, ..Default::default() }
    }

    /// Pixels per point, as the page's own density.
    fn pixels_per_point(&self) -> f32 {
        self.density
    }

    /// A frame is wanted: woken from the render thread's wait.
    pub fn wake(&mut self) {
        self.wants_frame = true;
    }

    /// Takes what was gathered since the last frame, leaving it empty.
    pub fn take_events(&mut self) -> (Vec<WindowEvent>, Vec<egui::Event>, egui::Modifiers, bool) {
        let window = std::mem::take(&mut self.window_events);
        let egui = std::mem::take(&mut self.egui_events);
        (window, egui, self.modifiers, self.focused)
    }

    /// The keys that came since the last frame, for `Input::on_key`.
    pub fn take_keys(&mut self) -> Vec<(KeyCode, bool, bool)> {
        std::mem::take(&mut self.keys)
    }

    /// A touch came down, moved, lifted or was cancelled. `pen` says whether it is a stylus;
    /// `pressure` 0..1 (a finger gives 1); `tilt` radians from upright, `orientation` the
    /// direction it leans; `eraser` whether the pen's other end is what touches.
    #[allow(clippy::too_many_arguments)]
    pub fn touch(
        &mut self,
        id: u64,
        phase: TouchPhase,
        x: f64,
        y: f64,
        pressure: f64,
        tilt: f64,
        orientation: f64,
        pen: bool,
        eraser: bool,
    ) {
        self.wants_frame = true;
        let before = self.pointers.get(&id).cloned().unwrap_or_default();
        let mut now = before.clone();
        now.pen = pen;
        now.eraser = eraser;
        now.at = (x, y);
        now.pressure = pressure.clamp(0.0, 1.0);
        now.tilt = tilt;
        now.orientation = orientation;
        now.down = matches!(phase, TouchPhase::Started | TouchPhase::Moved);
        if phase == TouchPhase::Started {
            now.began = Some(Instant::now());
        }
        self.pointers.insert(id, now.clone());
        // what the pointer is: the interface's own input asks (a finger may give a pressure too)
        crate::pen::pointer_kind(id, pen);
        crate::pen::eraser_kind(id, eraser);
        // the pen's lean, as the shell read it: radians from upright, and the direction it leans
        // (the same pair the desktop's pens give, so the strokes take it as they do there). It goes
        // with every touch, not only its landing, because a pen's lean changes while it draws — a
        // brush that follows the barrel turns as the hand does.
        if pen {
            crate::pen::keep_pose(id, (tilt as f32, orientation as f32), None);
        }
        // the pen left the screen: the brush cursor has no lean to follow any more (a second pen
        // still down is still the one heard from, so it stays)
        if pen && matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) && !self.any_pen_down() {
            crate::pen::pen_gone();
        }

        // the interface's own input: a winit touch, as the desktop's pen and touches give it.
        // This side stays in *window pixels*, as winit's own touches are (the canvas lays itself
        // out in window pixels: its view, its brushes and its hit tests all are), so the system's
        // pixels are passed on as they came. Only egui, below, works in points.
        let force = if pen { Some(Force::Normalized(now.pressure)) } else { None };
        self.window_events.push(WindowEvent::Touch(Touch {
            device_id: DeviceId::dummy(),
            phase,
            location: PhysicalPosition::new(x, y),
            force,
            id,
        }));

        // egui: a pointer's move, press or release (egui's pointer ids are its own), in points
        let pos = egui::pos2((x / f64::from(self.density)) as f32, (y / f64::from(self.density)) as f32);
        self.egui_events.push(egui::Event::PointerMoved(pos));
        let button = if pen && eraser { egui::PointerButton::Secondary } else { egui::PointerButton::Primary };
        match phase {
            TouchPhase::Started => {
                self.egui_events.push(egui::Event::PointerButton {
                    pos,
                    button,
                    pressed: true,
                    modifiers: self.modifiers,
                });
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                self.egui_events.push(egui::Event::PointerButton {
                    pos,
                    button,
                    pressed: false,
                    modifiers: self.modifiers,
                });
                // a finger's or a pen's lift: egui forgets it, as a mouse leaving would
                self.egui_events.push(egui::Event::PointerGone);
            }
            TouchPhase::Moved => {}
        }
    }

    /// A pen is near but not touching: hovering (its outline follows it).
    pub fn hover(&mut self, id: u64, x: f64, y: f64, tilt: f64, orientation: f64, eraser: bool) {
        self.wants_frame = true;
        let mut now = self.pointers.get(&id).cloned().unwrap_or_default();
        // a pointer that is already down has been heard from already: the shell says a mouse's
        // move twice, once as a touch and once as a hover, and the move is not said again (nor is
        // the pointer made to seem up while its button still is)
        let dragging = now.down;
        now.pen = true;
        now.eraser = eraser;
        now.at = (x, y);
        now.tilt = tilt;
        now.orientation = orientation;
        now.down = dragging;
        self.pointers.insert(id, now);
        // a pen near the screen leans too: the brush's outline turns with it as it comes into
        // reach, before it ever touches (the desktop's pens are read the same way, winit or not)
        crate::pen::keep_pose(id, (tilt as f32, orientation as f32), None);
        if dragging {
            return;
        }
        // the interface's cursor follows it (its brush outline is the canvas's own)
        let ppp = f64::from(self.density);
        self.egui_events.push(egui::Event::PointerMoved(egui::pos2((x / ppp) as f32, (y / ppp) as f32)));
    }

    /// A pen left the tablet's reach.
    pub fn hover_out(&mut self, id: u64) {
        self.wants_frame = true;
        self.pointers.remove(&id);
        self.egui_events.push(egui::Event::PointerGone);
        if !self.any_pen_down() {
            crate::pen::pen_gone();
        }
    }

    /// Whether a pen is down (it is the one the brush cursor follows while it draws).
    fn any_pen_down(&self) -> bool {
        self.pointers.values().any(|p| p.down && p.pen)
    }

    /// A key went down or up. `text` is what it types, when it types anything (a soft keyboard's
    /// characters come this way too).
    pub fn key(&mut self, named: Option<NamedKey>, code: Option<KeyCode>, character: Option<char>, pressed: bool, repeat: bool) {
        self.wants_frame = true;
        // the interface's own two (Space, R) are told apart from egui's: see `Input::on_key`
        if let Some(code) = code {
            self.keys.push((code, pressed, repeat));
        }
        // egui: the key's press or release, with the modifiers as they were while it was typed
        // (winit says a key before it says what is held with it)
        if let Some(key) = egui_key(named, character) {
            self.egui_events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: pressed && repeat,
                modifiers: self.modifiers,
            });
        }
        if pressed
            && !repeat
            && !self.modifiers.ctrl
            && !self.modifiers.mac_cmd
            && let Some(c) = character
        {
            self.egui_events.push(egui::Event::Text(c.to_string()));
        }
        // A modifier is held or let go as any other key is: the shell hands the keys over one at a
        // time and says nothing else of them, so what is held is built up here, key by key, and is
        // said when it changes. It is never emptied wholesale, as it was: the last event of a
        // keystroke is its release, so an emptied state left the app believing nothing was held
        // while the key still was (every Ctrl and Shift shortcut went, and a pen turning the canvas
        // with R let go of Shift at the first key it came with).
        let changed = match &named {
            Some(NamedKey::Shift) => held(&mut self.modifiers.shift, &mut self.winit_modifiers, ModifiersState::SHIFT, pressed),
            Some(NamedKey::Control) => {
                held(&mut self.modifiers.ctrl, &mut self.winit_modifiers, ModifiersState::CONTROL, pressed)
            }
            Some(NamedKey::Alt) => held(&mut self.modifiers.alt, &mut self.winit_modifiers, ModifiersState::ALT, pressed),
            Some(NamedKey::Meta) => {
                held(&mut self.modifiers.mac_cmd, &mut self.winit_modifiers, ModifiersState::SUPER, pressed)
            }
            _ => false,
        };
        // Ctrl is this platform's command key, as ⌘ is a Mac's: egui's `Modifiers::COMMAND` is taken
        // by `command` and never by `ctrl`, so the two are kept together (every shortcut the
        // interface has — Save, New, Open, Copy — asks for `Modifiers::COMMAND`)
        self.modifiers.command = self.modifiers.ctrl || self.modifiers.mac_cmd;
        if changed {
            self.window_events.push(WindowEvent::ModifiersChanged(self.winit_modifiers.into()));
        }
    }

    /// What a soft keyboard typed (its characters come without a key of their own).
    pub fn text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.wants_frame = true;
        for c in text.chars() {
            if c == '\u{8}' {
                // backspace
                self.egui_events.push(egui::Event::Key {
                    key: egui::Key::Backspace,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: self.modifiers,
                });
            } else {
                self.egui_events.push(egui::Event::Text(c.to_string()));
            }
        }
    }

    /// The window's size changed (the panel turned, or its window did).
    pub fn resized(&mut self, width: u32, height: u32) {
        self.wants_frame = true;
        self.window_events.push(WindowEvent::Resized(winit::dpi::PhysicalSize::new(width, height)));
    }

    /// The pointer scrolled: `at` is where it is (window pixels), `(x, y)` what it scrolled by.
    ///
    /// Nothing calls this yet. The shell registers a touch, a mouse, a hover and a key callback and
    /// no other (`napi_init.cpp`), and the one that would carry a scroll —
    /// `OH_NativeXComponent_RegisterUIInputEventCallback`, whose `ARKUI_UIINPUTEVENT_TYPE_AXIS`
    /// events are a wheel's or a trackpad's slides — is never registered. So a mouse's wheel and a
    /// two-finger slide reach the app not at all: the interface does not scroll, and the view
    /// neither pans nor zooms with them. What the shell must do is written up in the review; this
    /// side is ready for it.
    pub fn scroll(&mut self, at: (f64, f64), x: f32, y: f32) {
        self.wants_frame = true;
        // The interface's own input takes a wheel where the pointer is, and it learns where that is
        // from a move and from nothing else (`input.rs`, `CursorMoved`): a wheel comes with no move
        // of its own, so one is made here, as winit would have made it.
        self.window_events.push(WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(at.0, at.1),
        });
        self.egui_events.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(x, y),
            phase: egui::TouchPhase::Move,
            modifiers: self.modifiers,
            source: egui::MouseWheelSource::Trackpad,
        });
        self.window_events.push(WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(x as f64, y as f64)),
            phase: TouchPhase::Moved,
        });
    }

    /// The window gained or lost focus (the app went to the background).
    pub fn focused(&mut self, focused: bool) {
        self.focused = focused;
        self.wants_frame = true;
        self.window_events.push(WindowEvent::Focused(focused));
        if !focused {
            // every pointer let go: nothing stays down while the app is away
            for (id, pointer) in std::mem::take(&mut self.pointers) {
                if pointer.down {
                    self.touch(id, TouchPhase::Cancelled, pointer.at.0, pointer.at.1, 1.0, 0.0, 0.0, pointer.pen, pointer.eraser);
                }
            }
            self.egui_events.push(egui::Event::PointerGone);
        }
    }

    /// A file was dropped on the window (a picture opened with the app).
    pub fn dropped_file(&mut self, path: std::path::PathBuf) {
        self.wants_frame = true;
        self.window_events.push(WindowEvent::DroppedFile(path));
    }

    /// The window is to close (the app went away).
    pub fn close_requested(&mut self) {
        self.window_events.push(WindowEvent::CloseRequested);
    }

    /// Whether anything is still down (a stroke in progress).
    pub fn any_down(&self) -> bool {
        self.pointers.values().any(|p| p.down)
    }
}
