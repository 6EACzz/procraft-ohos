//! What the tablet's shell (the ArkTS page that shows the app) answers: a file or folder picked,
//! a message shown, the clipboard read and written, the soft keyboard shown and hidden.
//!
//! The interface asks on the app's own thread, which must not stop drawing; the shell runs on the
//! app's main thread and cannot be called back into from here. So each ask is queued, and the
//! shell takes what is queued as it comes round (see `napi_init.cpp`'s `asks`), answering each by
//! id. What was asked waits for its answer on the asking thread's own time: the interface keeps
//! drawing until it comes, and goes on without it if the shell says no.

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
    /// A picture onto the clipboard: its pixels written as a PNG at `file` in the app's own folder,
    /// which is what the shell copies from (a picture is copied out of a file, not out of pixels
    /// as words — and holding them here until the shell comes round would hold them twice).
    ClipboardWritePicture { width: u32, height: u32, file: std::path::PathBuf },
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
    /// When the ask stops being waited for: none for one asked from the shell's own thread, which
    /// has no one to wait for it. Past due means its asker has gone on without it, and the shell
    /// need not be told about it at all (see [`take`]).
    waited_until: Option<std::time::Instant>,
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
/// What the shell took and has not answered.
static TAKEN: Mutex<Vec<Queued>> = Mutex::new(Vec::new());
/// The next ask's id.
static NEXT: Mutex<u64> = Mutex::new(1);

/// How long an ask waits for its answer. One a person answers — a pick, a folder's files read — is
/// waited on for as long as a person may take; the rest the shell answers at once or not at all,
/// and the thread asking is the app's own, which has a canvas to keep drawing.
const WAITED_FOR_A_PERSON: std::time::Duration = std::time::Duration::from_secs(600);
const WAITED_FOR_THE_SHELL: std::time::Duration = std::time::Duration::from_secs(15);

/// The most asks the shell may take and leave unanswered. Every asker clears its own entry as it
/// stops waiting (see [`forget`]), so reaching this means the shell has stopped taking answers
/// altogether: the oldest go rather than this growing for as long as the app runs.
const TAKEN_AT_MOST: usize = 64;

/// How long `wanted` is waited for.
fn waited_for(wanted: &Wanted) -> std::time::Duration {
    match wanted {
        Wanted::Pick(_) | Wanted::ClipboardRead => WAITED_FOR_A_PERSON,
        Wanted::Say { .. }
        | Wanted::ClipboardWritePicture { .. }
        | Wanted::ClipboardWriteText { .. }
        | Wanted::Keyboard { .. } => WAITED_FOR_THE_SHELL,
    }
}

/// The next id to ask under.
fn next_id() -> u64 {
    let mut next = NEXT.lock().unwrap_or_else(|e| e.into_inner());
    let id = *next;
    *next += 1;
    id
}

/// Queues `wanted`, and waits for the shell's answer (which the shell gives from its own thread
/// through `answer`): none if it never came, which is not the same as the shell saying no.
fn ask(wanted: Wanted) -> Option<Answered> {
    let (done, answer) = channel();
    let (id, what) = (next_id(), kind(&wanted));
    let waited = waited_for(&wanted);
    let waited_until = std::time::Instant::now() + waited;
    QUEUE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(Queued { id, wanted, answer: done, waited_until: Some(waited_until) });
    // (the app's own loop wakes the shell: see `ohos_main`'s asks)
    let answered = wait(answer, waited, what);
    // answered or not, nothing waits for this one any more: a shell that never answers must not
    // leave its entry behind (see `TAKEN_AT_MOST`)
    forget(id);
    if let Some(answered) = &answered
        && !answered.done
    {
        log::warn!("the shell couldn't do {what}: going on without it");
    }
    answered
}

/// Queues `wanted` without waiting for the shell to answer it: for a caller on the shell's own
/// thread (the app failing to start), which is the only thread that could show it — waiting there
/// would wait on itself.
fn ask_unwaited(wanted: Wanted) {
    let (done, answer) = channel();
    // (nothing ever reads it: the answer is dropped with the queue entry)
    drop(answer);
    let id = next_id();
    // (no time: nothing waits for it, so nothing ever stops waiting for it either)
    QUEUE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(Queued { id, wanted, answer: done, waited_until: None });
}

/// Whether an ask's asker has gone on without its answer: true once its own wait is over, whether
/// that was because the shell answered it or because the wait timed out.
fn given_up(queued: &Queued) -> bool {
    queued.waited_until.is_some_and(|until| until <= std::time::Instant::now())
}

/// Waits for an answer, going on without one if the shell never gives it: an ask the shell leaves
/// unanswered must not hold the thread that asked past what it was willing to wait.
fn wait(answer: Receiver<Answered>, waited: std::time::Duration, what: &str) -> Option<Answered> {
    match answer.recv_timeout(waited) {
        Ok(answered) => Some(answered),
        Err(_) => {
            log::warn!("the shell didn't answer {what} within {}s: going on without it", waited.as_secs());
            None
        }
    }
}

/// Forgets ask `id`: what no longer waits for an answer has nothing to keep, whether the shell has
/// taken the ask yet or not.
fn forget(id: u64) {
    QUEUE.lock().unwrap_or_else(|e| e.into_inner()).retain(|queued| queued.id != id);
    TAKEN.lock().unwrap_or_else(|e| e.into_inner()).retain(|queued| queued.id != id);
}

/// Takes what the shell is to do now (the shell's own thread, each frame): each with its id, and
/// the shape the shell reads (`kind` and `json`).
pub fn take() -> Vec<(u64, String, String)> {
    // what is queued moves to what is waiting for an answer
    let queued: Vec<Queued> = {
        let mut queue = QUEUE.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *queue)
    };
    let mut taken: Vec<(u64, String, String)> = Vec::with_capacity(queued.len());
    let mut waiting: Vec<Queued> = Vec::with_capacity(queued.len());
    for queued in queued {
        let named = kind(&queued.wanted);
        // one whose asker stopped waiting is one the shell need not be told about at all
        if given_up(&queued) {
            log::debug!("dropping the {named} nobody waits for");
            continue;
        }
        taken.push((queued.id, named.to_owned(), details(&queued.wanted)));
        waiting.push(queued);
    }
    let mut kept = TAKEN.lock().unwrap_or_else(|e| e.into_inner());
    kept.extend(waiting);
    // (read again here, under the lock [`forget`] takes: an asker that stopped waiting between the
    // two locks above would otherwise be kept for ever)
    kept.retain(|queued| !given_up(queued));
    if kept.len() > TAKEN_AT_MOST {
        // the shell has stopped answering: the oldest asks are the ones nothing will come back to
        let over = kept.len() - TAKEN_AT_MOST;
        kept.drain(..over);
        log::warn!("the shell left {} asks unanswered: forgotten the oldest {over}", kept.len() + over);
    }
    taken
}

/// What an ask is, as the shell and the log name it.
fn kind(wanted: &Wanted) -> &'static str {
    match wanted {
        Wanted::Pick(_) => "pick",
        Wanted::Say { .. } => "say",
        Wanted::ClipboardRead => "clipboard-read",
        Wanted::ClipboardWritePicture { .. } => "clipboard-write-picture",
        Wanted::ClipboardWriteText { .. } => "clipboard-write-text",
        Wanted::Keyboard { .. } => "keyboard",
    }
}

/// The shell's answer to ask `id`: the paths picked, the clipboard's contents, or nothing.
pub fn answer(id: u64, answered: Answered) {
    let mut taken = TAKEN.lock().unwrap_or_else(|e| e.into_inner());
    let at = taken.iter().position(|q| q.id == id);
    if let Some(at) = at {
        let queued = taken.remove(at);
        // (no one left to tell: its asker went on without it)
        let _ = queued.answer.send(answered);
    }
}

/// An ask's own details, as JSON, in the shape the shell reads for its [`kind`].
fn details(wanted: &Wanted) -> String {
    match wanted {
        Wanted::Pick(picked) => {
            let filters: Vec<String> = picked
                .filters
                .iter()
                .map(|(name, endings)| {
                    // (each ending too: a file kind's own name is anything the system allows)
                    let endings: Vec<String> = endings.iter().map(|e| format!("\"{}\"", escape(e))).collect();
                    format!("{{\"name\":\"{}\",\"endings\":[{}]}}", escape(name), endings.join(","))
                })
                .collect();
            let name = picked.name.as_deref().map_or_else(|| "null".to_owned(), |n| format!("\"{}\"", escape(n)));
            format!(
                "{{\"title\":\"{}\",\"name\":{},\"filters\":[{}],\"save\":{},\"folder\":{},\"several\":{}}}",
                escape(&picked.title),
                name,
                filters.join(","),
                picked.save,
                picked.folder,
                picked.several
            )
        }
        Wanted::Say { title, text, error } => {
            format!("{{\"title\":\"{}\",\"text\":\"{}\",\"error\":{}}}", escape(title), escape(text), error)
        }
        Wanted::ClipboardRead => "{}".to_owned(),
        Wanted::ClipboardWritePicture { width, height, file } => {
            // (a picture is copied from a file, not from pixels as words: the pixels went to the
            // file above, and this is only where it is)
            format!("{{\"width\":{width},\"height\":{height},\"file\":\"{}\"}}", escape(&file.to_string_lossy()))
        }
        Wanted::ClipboardWriteText { text } => format!("{{\"text\":\"{}\"}}", escape(text)),
        Wanted::Keyboard { shown } => format!("{{\"shown\":{shown}}}"),
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

/// A file or folder, as the interface asked for one.
pub fn pick(ask: Picked) -> PickedPaths {
    ask_wanted(Wanted::Pick(ask))
}

/// A file or folder, as the interface asked for one (waits for the shell).
fn ask_wanted(wanted: Wanted) -> PickedPaths {
    let Some(answered) = ask(wanted) else { return PickedPaths::new() };
    if !answered.done {
        // (the shell couldn't ask: what it picked is nothing, which reads as cancelled)
        return PickedPaths::new();
    }
    answered.paths.into_iter().map(std::path::PathBuf::from).collect()
}

/// Shows `text`, queued and not waited for: for the shell's own thread (see [`ask_unwaited`]).
pub fn say_unwaited(title: &str, text: &str, error: bool) {
    ask_unwaited(Wanted::Say { title: title.to_owned(), text: text.to_owned(), error });
}

/// Shows `text` with an OK button, waiting until it is seen — or until the shell says it can't,
/// which leaves the text in the log (the only place left to say it).
pub fn say(title: &str, text: &str, error: bool) {
    let _ = ask(Wanted::Say { title: title.to_owned(), text: text.to_owned(), error });
}

/// What the clipboard holds.
pub fn clipboard_read() -> Clipboard {
    let Some(answered) = ask(Wanted::ClipboardRead) else { return Clipboard::Nothing };
    match answered.clipboard {
        Some(ClipboardAnswer::Picture { width, height, rgba }) => Clipboard::Picture((width, height, rgba)),
        Some(ClipboardAnswer::Files(files)) => Clipboard::Files(files.into_iter().map(std::path::PathBuf::from).collect()),
        Some(ClipboardAnswer::Text(text)) => Clipboard::Text(text),
        _ => Clipboard::Nothing,
    }
}

/// `width` by `height` of straight RGBA8 pixels written as a PNG in the app's own folder, named so
/// that two copies at once don't write into one another; where it was written, or none (said in the
/// log) when it couldn't be.
fn write_picture(width: u32, height: u32, rgba: &[u8]) -> Option<std::path::PathBuf> {
    if width == 0 || height == 0 || rgba.len() != width as usize * height as usize * 4 {
        log::warn!("a picture of {width}x{height} isn't {} bytes of pixels", rgba.len());
        return None;
    }
    let dir = crate::files::app_dir().join("clipboard");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::warn!("the clipboard's own folder {}: {e}", dir.display());
        return None;
    }
    let path = dir.join(format!("picture-{}.png", next_id()));
    let written = std::fs::File::create(&path).map_err(|e| e.to_string()).and_then(|out| -> Result<(), String> {
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(out), width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        // (a clipboard's picture is read back at once: made quickly rather than small)
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(rgba).map_err(|e| e.to_string())?;
        // (finished rather than merely dropped: a PNG is whole only once its end is written, and
        // `finish` flushes the file too, which a dropped writer doesn't report)
        writer.finish().map_err(|e| e.to_string())
    });
    match written {
        Ok(()) => Some(path),
        Err(e) => {
            log::warn!("the picture for the clipboard at {}: {e}", path.display());
            let _ = std::fs::remove_file(&path);
            None
        }
    }
}

/// A picture onto the clipboard: written as a PNG in the app's own folder, which the shell copies
/// into the pasteboard, and which this then removes.
pub fn clipboard_write_picture(width: u32, height: u32, rgba: &[u8]) {
    let Some(file) = write_picture(width, height, rgba) else { return };
    let answered = ask(Wanted::ClipboardWritePicture { width, height, file: file.clone() });
    // (the shell has copied it by then, or given up on it: either way the file is this side's)
    let _ = std::fs::remove_file(&file);
    if answered.is_some_and(|answered| !answered.done) {
        // said without waiting: this is the thread that asked, and waiting here would wait on the
        // very answer it hasn't finished with
        say_unwaited("ProCraft", "The picture couldn't be copied.", true);
    }
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
        let read = crate::files::read_path(std::path::Path::new(path));
        let rgba = read.ok().and_then(|bytes| crate::files::png_rgba(&bytes));
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
        // (the shell stages what it copied into the app's own folder and sends paths: one that is
        // still a URI is one it couldn't stage, and reading it as a path would only fail later)
        let files: Vec<String> = rest
            .lines()
            .filter(|line| !line.is_empty())
            .filter(|line| {
                if crate::files::openable(std::path::Path::new(line)) {
                    return true;
                }
                log::warn!("the shell sent a clipboard file that isn't a path: {line}");
                false
            })
            .map(str::to_owned)
            .collect();
        return Some(ClipboardAnswer::Files(files));
    }
    if let Some(rest) = text.strip_prefix("text:") {
        return Some(ClipboardAnswer::Text(rest.to_owned()));
    }
    None
}
