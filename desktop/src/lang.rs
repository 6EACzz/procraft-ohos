//! The interface's language: English as it is written in the code, and Chinese beside it.
//!
//! Every word the interface draws goes through [`tr`] (or [`tr_dynamic`] for words with something
//! filled into them), so a string is written once in English where it is used and read here. A
//! string the Chinese table doesn't have stays English: the interface is never blank, and a
//! half-translated build still reads.
//!
//! The table is keyed by the English text itself, not by an invented id: that keeps the code
//! readable (a reviewer sees the words, not `ui.gallery.title`), and a missing entry is obvious.

use std::sync::atomic::{AtomicU8, Ordering};

/// The languages the interface is drawn in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Language {
    /// English, as the code writes it.
    #[default]
    English,
    /// Simplified Chinese.
    Chinese,
}

impl Language {
    /// Every language, in the order the settings page lists them.
    pub const ALL: [Language; 2] = [Language::English, Language::Chinese];

    /// The language's name in itself, which a settings page shows whatever the interface's own.
    pub fn name(self) -> &'static str {
        match self {
            Language::English => "English",
            Language::Chinese => "中文（简体）",
        }
    }

    /// The language as it is kept in the app's folder.
    pub fn key(self) -> &'static str {
        match self {
            Language::English => "en",
            Language::Chinese => "zh-Hans",
        }
    }

    /// The language kept under `key`, if it is one of them.
    pub fn of_key(key: &str) -> Option<Language> {
        Language::ALL.into_iter().find(|l| l.key() == key)
    }
}

/// The language being drawn, as the app set it (a settings page's choice).
static CURRENT: AtomicU8 = AtomicU8::new(0);

/// The language the interface is drawn in now.
pub fn language() -> Language {
    match CURRENT.load(Ordering::Relaxed) {
        1 => Language::Chinese,
        _ => Language::English,
    }
}

/// The interface is drawn in `language` from the next frame on.
pub fn set_language(language: Language) {
    CURRENT.store(match language {
        Language::English => 0,
        Language::Chinese => 1,
    }, Ordering::Relaxed);
}

/// `text` in the interface's language: itself in English, its Chinese in Chinese, and itself
/// again for anything the table hasn't got.
pub fn tr(text: &'static str) -> &'static str {
    match language() {
        Language::English => text,
        Language::Chinese => zh(text).unwrap_or(text),
    }
}

/// `text` in the interface's language, borrowed rather than built: what the interface draws goes
/// through here, so drawing a word costs nothing while the interface is English.
pub fn tr_cow(text: &str) -> std::borrow::Cow<'_, str> {
    match language() {
        Language::English => std::borrow::Cow::Borrowed(text),
        // an entry is the interface's own words, in the table for as long as the app runs
        Language::Chinese => std::borrow::Cow::Borrowed(zh(text).unwrap_or(text)),
    }
}

/// `text` in the interface's language, with `fills` put where it has `{}` (the pieces are
/// themselves translated, and an entry keeps their order).
pub fn tr_fill(text: &'static str, fills: &[&str]) -> String {
    let pattern = tr(text);
    let mut said = String::with_capacity(pattern.len() + 16);
    let mut pieces = fills.iter();
    let mut rest = pattern;
    while let Some(at) = rest.find("{}") {
        said.push_str(&rest[..at]);
        said.push_str(pieces.next().copied().unwrap_or(""));
        rest = &rest[at + 2..];
    }
    said.push_str(rest);
    said
}

/// Words the code builds itself (a name, a count, a message an outcome made) in the interface's
/// language: the table's entry when the whole of it is one of its own (a sentence a result built),
/// else the words as they are — those are the user's own (an artwork's name, a file's), or a
/// piece already translated.
pub fn tr_dynamic(text: &str) -> String {
    match language() {
        Language::English => text.to_owned(),
        Language::Chinese => match zh_dynamic(text) {
            Some(said) => said,
            None => text.to_owned(),
        },
    }
}

pub mod zh;

use zh::{zh, zh_dynamic};
