//! What the tablet's shell (the ArkTS page that shows the app) answers: a file or folder picked,
//! a message shown, the clipboard read and written, the soft keyboard shown and hidden.
//!
//! The interface asks on the app's own thread, which must not stop drawing; the shell runs on the
//! app's main thread and cannot be called back into from here. So each ask is queued, and the
//! shell takes what is queued as it comes round (see `napi_init.cpp`'s `asks`), answering each by
//! id. What was asked waits for its answer on the asking thread's own time: the interface keeps
//! drawing until it comes, and goes on without it if the shell says no.

#![allow(unsafe_code)]

use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};

use crate::platform::{Clipboard, Picked, PickedPaths};

/// What the shell is asked to do.
#[derive(Clone, Debug)]
pub enum Wanted {
    /// A file or folder, as the interface asked for one.
    Pick(Picked),
    /// A message with an OK button.
    Say { title: String, text: String, error: bool },
    /// What the clipboard holds.
    ClipboardRead,
    /// A picture onto the clipboard (straight RGBA).
    ClipboardWritePicture { width: u32, height: u32, rgba: Vec<u8> },
    /// Words onto the clipboard.
    ClipboardWriteText { text: String },
    /// The soft keyboard shown (a text field took the focus) or hidden.
    Keyboard { shown: bool },
}

/// One ask, waiting for the shell to take it.
struct Queued {
    id: u64,
    wanted: Wanted,
    /// Where its answer goes (a Pick's paths, a Say's nothing, the clipboard's contents).
    answer: Sender<Answered>,
}

/// What the shell answered.
#[derive(Clone, Debug, Default)]
pub struct Answered {
    /// The paths picked (none: cancelled).
    pub paths: Vec<String>,
    /// What the clipboard held, as the shell read it.
    pub clipboard: Option<ClipboardAnswer>,
    /// Whether the shell could do it at all.
    pub done: bool,
}

/// The clipboard, as the shell hands it over: a picture's pixels (straight RGBA), its files, or
/// words.
#[derive(Clone, Debug)]
pub enum ClipboardAnswer {
    Picture { width: u32, height: u32, rgba: Vec<u8> },
    Files(Vec<String>),
    Text(String),
    Nothing,
}

/// What the shell is to do, in the order it was asked.
static QUEUE: Mutex<Vec<Queued>> = Mutex::new(Vec::new());
/// The next ask's id.
static NEXT: Mutex<u64> = Mutex::new(1);

/// Queues `wanted`, and waits for the shell's answer (which the shell gives from its own thread
/// through `answer`).
fn ask(wanted: Wanted) -> Answered {
    let (done, answer) = channel();
    let id = {
        let mut next = NEXT.lock().unwrap_or_else(|e| e.into_inner());
        let id = *next;
        *next += 1;
        id
    };
    QUEUE.lock().unwrap_or_else(|e| e.into_inner()).push(Queued { id, wanted, answer: done });
    // (the app's own loop wakes the shell: see `ohos_main`'s asks)
    wait(answer)
}

/// Queues `wanted` without waiting for the shell to answer it: for a caller on the shell's own
/// thread (the app failing to start), which is the only thread that could show it — waiting there
/// would wait on itself.
fn ask_unwaited(wanted: Wanted) {
    let (done, answer) = channel();
    // (nothing ever reads it: the answer is dropped with the queue entry)
    drop(answer);
    let id = {
        let mut next = NEXT.lock().unwrap_or_else(|e| e.into_inner());
        let id = *next;
        *next += 1;
        id
    };
    QUEUE.lock().unwrap_or_else(|e| e.into_inner()).push(Queued { id, wanted, answer: done });
}

/// Waits for an answer, going on without one if the shell never gives it.
fn wait(answer: Receiver<Answered>) -> Answered {
    match answer.recv_timeout(std::time::Duration::from_secs(600)) {
        Ok(answered) => answered,
        Err(_) => Answered::default(),
    }
}

/// Takes what the shell is to do now (the shell's own thread, each frame): each with its id, and
/// the shape the shell reads (`kind` and `json`).
pub fn take() -> Vec<(u64, String, String)> {
    // what is queued moves to what is waiting for an answer
    let queued: Vec<Queued> = {
        let mut queue = QUEUE.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *queue)
    };
    let taken: Vec<(u64, String, String)> = queued
        .iter()
        .map(|queued| {
            let (kind, json) = describe(&queued.wanted);
            (queued.id, kind, json)
        })
        .collect();
    TAKEN.lock().unwrap_or_else(|e| e.into_inner()).extend(queued);
    taken
}

/// What was taken and not yet answered.
static TAKEN: Mutex<Vec<Queued>> = Mutex::new(Vec::new());

/// The shell's answer to ask `id`: the paths picked, the clipboard's contents, or nothing.
pub fn answer(id: u64, answered: Answered) {
    let mut taken = TAKEN.lock().unwrap_or_else(|e| e.into_inner());
    let at = taken.iter().position(|q| q.id == id);
    if let Some(at) = at {
        let queued = taken.remove(at);
        let _ = queued.answer.send(answered);
    }
}

/// An ask as the shell reads it: what it is (`pick`, `say`, `clipboard-read`, ...) and its own
/// details, as JSON.
fn describe(wanted: &Wanted) -> (String, String) {
    match wanted {
        Wanted::Pick(picked) => {
            let filters: Vec<String> = picked
                .filters
                .iter()
                .map(|(kind, endings)| {
                    // (each ending too: a file kind's own name is anything the system allows)
                    let endings: Vec<String> = endings.iter().map(|e| format!("\"{}\"", escape(e))).collect();
                    format!("{{\"name\":\"{}\",\"endings\":[{}]}}", escape(kind), endings.join(","))
                })
                .collect();
            let name = picked.name.as_deref().map_or_else(|| "null".to_owned(), |n| format!("\"{}\"", escape(n)));
            (
                "pick".to_owned(),
                format!(
                    "{{\"title\":\"{}\",\"name\":{},\"filters\":[{}],\"save\":{},\"folder\":{},\"several\":{}}}",
                    escape(&picked.title),
                    name,
                    filters.join(","),
                    picked.save,
                    picked.folder,
                    picked.several
                ),
            )
        }
        Wanted::Say { title, text, error } => (
            "say".to_owned(),
            format!("{{\"title\":\"{}\",\"text\":\"{}\",\"error\":{}}}", escape(title), escape(text), error),
        ),
        Wanted::ClipboardRead => ("clipboard-read".to_owned(), "{}".to_owned()),
        Wanted::ClipboardWritePicture { width, height, rgba } => (
            "clipboard-write-picture".to_owned(),
            format!("{{\"width\":{width},\"height\":{height},\"bytes\":{}}}", rgba.len()),
        ),
        Wanted::ClipboardWriteText { text } => {
            ("clipboard-write-text".to_owned(), format!("{{\"text\":\"{}\"}}", escape(text)))
        }
        Wanted::Keyboard { shown } => ("keyboard".to_owned(), format!("{{\"shown\":{shown}}}")),
    }
}

/// Text made safe for the JSON the shell reads.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// The picture the clipboard holds, as straight RGBA.
pub fn picture_of(answer: &ClipboardAnswer) -> Option<(u32, u32, Vec<u8>)> {
    match answer {
        ClipboardAnswer::Picture { width, height, rgba } => Some((*width, *height, rgba.clone())),
        _ => None,
    }
}

/// A file or folder, as the interface asked for one.
pub fn pick(ask: Picked) -> PickedPaths {
    ask_wanted(Wanted::Pick(ask))
}

/// A file or folder, as the interface asked for one (waits for the shell).
fn ask_wanted(wanted: Wanted) -> PickedPaths {
    let answered = ask(wanted);
    answered.paths.into_iter().map(std::path::PathBuf::from).collect()
}

/// A message with an OK button, waiting until it is seen.
/// Shows `text`, queued and not waited for: for the shell's own thread (see [`ask_unwaited`]).
pub fn say_unwaited(title: &str, text: &str, error: bool) {
    ask_unwaited(Wanted::Say { title: title.to_owned(), text: text.to_owned(), error });
}

/// Shows `text` with an OK button, waiting until it is seen.
pub fn say(title: &str, text: &str, error: bool) {
    let _ = ask(Wanted::Say { title: title.to_owned(), text: text.to_owned(), error });
}

/// What the clipboard holds.
pub fn clipboard_read() -> Clipboard {
    let answered = ask(Wanted::ClipboardRead);
    match answered.clipboard {
        Some(ClipboardAnswer::Picture { width, height, rgba }) => Clipboard::Picture((width, height, rgba)),
        Some(ClipboardAnswer::Files(files)) => Clipboard::Files(files.into_iter().map(std::path::PathBuf::from).collect()),
        Some(ClipboardAnswer::Text(text)) => Clipboard::Text(text),
        _ => Clipboard::Nothing,
    }
}

/// A picture onto the clipboard.
pub fn clipboard_write_picture(width: u32, height: u32, rgba: &[u8]) {
    let _ = ask(Wanted::ClipboardWritePicture { width, height, rgba: rgba.to_vec() });
}

/// Words onto the clipboard.
pub fn clipboard_write_text(text: &str) {
    let _ = ask(Wanted::ClipboardWriteText { text: text.to_owned() });
}

/// The soft keyboard, shown when a text field takes the focus.
pub fn keyboard(shown: bool) {
    let _ = ask(Wanted::Keyboard { shown });
}

/// The clipboard's contents as the shell sends them: `picture:WxH:<rgba as hex>`, `files:` and
/// the paths (one a line), `text:` and the words, or `nothing`.
pub fn clipboard_of(text: &str) -> Option<ClipboardAnswer> {
    if text == "nothing" || text.is_empty() {
        return Some(ClipboardAnswer::Nothing);
    }
    // a picture the shell wrote into the app's own folder (a clipboard's, which is a PNG):
    // read here, so the shell never hands over megabytes of pixels as words
    if let Some(path) = text.strip_prefix("picture-file:") {
        let rgba = std::fs::read(path).ok().and_then(|bytes| crate::files::png_rgba(&bytes));
        let _ = std::fs::remove_file(path);
        return match rgba {
            Some((width, height, rgba)) => Some(ClipboardAnswer::Picture { width, height, rgba }),
            None => Some(ClipboardAnswer::Nothing),
        };
    }
    if let Some(rest) = text.strip_prefix("picture:") {
        let mut parts = rest.splitn(3, ':');
        let width = parts.next()?.parse().ok()?;
        let height = parts.next()?.parse().ok()?;
        let hex = parts.next()?;
        let rgba = hex.as_bytes().chunks(2).filter_map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()).collect();
        return Some(ClipboardAnswer::Picture { width, height, rgba });
    }
    if let Some(rest) = text.strip_prefix("files:") {
        return Some(ClipboardAnswer::Files(rest.lines().map(str::to_owned).filter(|l| !l.is_empty()).collect()));
    }
    if let Some(rest) = text.strip_prefix("text:") {
        return Some(ClipboardAnswer::Text(rest.to_owned()));
    }
    None
}
