//! What the app asks of the system around it, in one place: a file or folder picked, a message
//! shown, a picture put on the clipboard. On the desktop these are the system's own dialogs and
//! clipboard (`rfd`, `arboard`); on a tablet (HarmonyOS) the interface has no such dialogs of its
//! own, so each is handed to the shell that runs the app (ArkTS, over N-API) and answered when it
//! can: a file picked, a message seen.
//!
//! Each function here is what the interface calls; which system answers is the platform's (see
//! `platform.rs` in the crate that builds it).

/// Where a file or folder comes from, as the interface asks for one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Picked {
    /// The dialog's title ("Save", "Import", "Share as PNG").
    pub title: String,
    /// What the file is called when the dialog first shows (a Save's name), if anything.
    pub name: Option<String>,
    /// The kinds offered, by name and endings (`("Pictures", ["png", "jpg"])`).
    pub filters: Vec<(String, Vec<String>)>,
    /// Whether it saves (a new file's name) rather than opens one.
    pub save: bool,
    /// Whether a folder is picked rather than a file.
    pub folder: bool,
    /// Whether several files may be picked at once.
    pub several: bool,
}

impl Picked {
    pub fn title(mut self, title: impl Into<String>) -> Picked {
        self.title = title.into();
        self
    }

    pub fn name(mut self, name: impl Into<String>) -> Picked {
        self.name = Some(name.into());
        self
    }

    pub fn filter(mut self, kind: impl Into<String>, endings: &[&str]) -> Picked {
        self.filters.push((kind.into(), endings.iter().map(|e| (*e).to_owned()).collect()));
        self
    }

    pub fn saving(mut self) -> Picked {
        self.save = true;
        self
    }

    pub fn folders(mut self) -> Picked {
        self.folder = true;
        self
    }

    pub fn many(mut self) -> Picked {
        self.several = true;
        self
    }
}

/// What the platform answers: the paths picked (none: cancelled).
pub type PickedPaths = Vec<std::path::PathBuf>;

/// Asks the system for a file or folder, waiting for the answer.
pub fn pick(ask: Picked) -> PickedPaths {
    system::pick(ask)
}

/// Shows `text` with an OK button, waiting until it is seen (or the system took it).
pub fn say(title: &str, text: &str, error: bool) {
    system::say(title, text, error)
}

/// A picture on the clipboard, straight RGBA.
pub type ClipboardImage = (u32, u32, Vec<u8>);

/// What the system's clipboard holds, as far as the app can use it.
pub enum Clipboard {
    /// A picture, straight RGBA.
    Picture(ClipboardImage),
    /// Picture files copied in a file manager.
    Files(Vec<std::path::PathBuf>),
    /// Words.
    Text(String),
    Nothing,
}

/// Reads what the system's clipboard holds.
pub fn clipboard_read() -> Clipboard {
    system::clipboard_read()
}

/// Writes a picture to the system's clipboard (straight RGBA).
pub fn clipboard_write_picture(width: u32, height: u32, rgba: &[u8]) {
    system::clipboard_write_picture(width, height, rgba)
}

/// Writes words to the system's clipboard.
pub fn clipboard_write_text(text: &str) {
    system::clipboard_write_text(text)
}

/// The soft keyboard is to show, a field of the interface being typed in, or to go.
///
/// The desktop needs nothing of it: its keyboard is the one on the desk, and a window that needs
/// no on-screen one never says so. A tablet's is the system's, shown over its own window, and the
/// shell shows and hides it.
pub fn keyboard(shown: bool) {
    system::keyboard(shown)
}

/// The platform's answers: the desktop's own dialogs and clipboard, or the shell's.
mod system {
    use super::{Clipboard, Picked, PickedPaths};
    #[cfg(not(target_env = "ohos"))]
    use super::ClipboardImage;

    #[cfg(not(target_env = "ohos"))]
    pub fn pick(ask: Picked) -> PickedPaths {
        let mut dialog = rfd::FileDialog::new().set_title(&ask.title);
        for (kind, endings) in &ask.filters {
            let endings: Vec<&str> = endings.iter().map(String::as_str).collect();
            dialog = dialog.add_filter(kind, &endings);
        }
        if let Some(name) = &ask.name {
            dialog = dialog.set_file_name(name);
        }
        if ask.folder {
            return dialog.pick_folder().into_iter().collect();
        }
        if ask.save {
            return dialog.save_file().into_iter().collect();
        }
        if ask.several {
            return dialog.pick_files().unwrap_or_default();
        }
        dialog.pick_file().into_iter().collect()
    }

    #[cfg(not(target_env = "ohos"))]
    pub fn say(title: &str, text: &str, error: bool) {
        use rfd::{MessageDialog, MessageLevel};
        let level = if error { MessageLevel::Error } else { MessageLevel::Info };
        MessageDialog::new().set_level(level).set_title(title).set_description(text).show();
    }

    #[cfg(not(target_env = "ohos"))]
    pub fn clipboard_read() -> Clipboard {
        let Ok(mut clipboard) = arboard::Clipboard::new() else { return Clipboard::Nothing };
        if let Ok(image) = clipboard.get_image() {
            return Clipboard::Picture((image.width as u32, image.height as u32, image.bytes.into_owned()));
        }
        if let Ok(text) = clipboard.get_text()
            && !text.is_empty()
        {
            return Clipboard::Text(text);
        }
        Clipboard::Nothing
    }

    #[cfg(not(target_env = "ohos"))]
    pub fn clipboard_write_picture(width: u32, height: u32, rgba: &[u8]) {
        let Ok(mut clipboard) = arboard::Clipboard::new() else { return };
        let image: ClipboardImage = (width, height, rgba.to_vec());
        let (width, height, rgba) = image;
        let _ = clipboard.set_image(arboard::ImageData {
            width: width as usize,
            height: height as usize,
            bytes: std::borrow::Cow::Owned(rgba),
        });
    }

    #[cfg(not(target_env = "ohos"))]
    pub fn clipboard_write_text(text: &str) {
        if let Ok(mut clipboard) = arboard::Clipboard::new() {
            let _ = clipboard.set_text(text.to_owned());
        }
    }

    /// Nothing to show or hide: the desktop's keyboard is not the window's to put away, and a
    /// field being typed in takes its letters from the keys of the machine.
    #[cfg(not(target_env = "ohos"))]
    pub fn keyboard(_shown: bool) {}

    // On HarmonyOS the shell (ArkTS) owns the dialogs and the clipboard: each is asked over N-API
    // and answered when the system hands it back (see `ohos_platform.rs`).
    #[cfg(target_env = "ohos")]
    pub fn pick(ask: Picked) -> PickedPaths {
        crate::ohos_platform::pick(ask)
    }

    #[cfg(target_env = "ohos")]
    pub fn say(title: &str, text: &str, error: bool) {
        crate::ohos_platform::say(title, text, error)
    }

    #[cfg(target_env = "ohos")]
    pub fn clipboard_read() -> Clipboard {
        crate::ohos_platform::clipboard_read()
    }

    #[cfg(target_env = "ohos")]
    pub fn clipboard_write_picture(width: u32, height: u32, rgba: &[u8]) {
        crate::ohos_platform::clipboard_write_picture(width, height, rgba)
    }

    #[cfg(target_env = "ohos")]
    pub fn clipboard_write_text(text: &str) {
        crate::ohos_platform::clipboard_write_text(text)
    }

    #[cfg(target_env = "ohos")]
    pub fn keyboard(shown: bool) {
        crate::ohos_platform::keyboard(shown)
    }
}
