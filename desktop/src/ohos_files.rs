//! Where the app may keep its files on the tablet, as the system itself names it. Its files
//! directory is the only place it can write: nothing under `$HOME` is (`/storage/Users/…`), and
//! the executable it runs as is `/system/bin/appspawn` (the app is a process it forked), so the
//! sandbox can't be found from either — only the system's own call can say.

#![allow(unsafe_code)]

use std::path::PathBuf;

/// The system's `FileManagement_ErrCode`/`AbilityRuntime_ErrorCode` for "it worked".
const NO_ERROR: i32 = 0;

/// The room given to the system's answer: its paths are a few dozen bytes.
const ROOM: usize = 4096;

#[link(name = "ability_runtime")]
unsafe extern "C" {
    /// `OH_AbilityRuntime_ApplicationContextGetFilesDir` (Core File Kit, since API 16): the
    /// application's own files directory, written into `buffer` (`size` bytes, the length written
    /// put in `written`).
    #[link_name = "OH_AbilityRuntime_ApplicationContextGetFilesDir"]
    fn application_files_dir(buffer: *mut std::ffi::c_char, size: i32, written: *mut i32) -> i32;
}

/// The application's own files directory, as the system says: none if it won't (an old system
/// without the call, or one that has no context yet).
pub fn files_dir() -> Option<PathBuf> {
    let mut buffer = vec![0u8; ROOM];
    let mut written = 0i32;
    // SAFETY: the system's own call, given a buffer of the room it is told of, live and writable
    let code = unsafe { application_files_dir(buffer.as_mut_ptr().cast(), ROOM as i32, &mut written) };
    if code != NO_ERROR || written <= 0 {
        log::warn!("the system doesn't say where the app's files go ({code})");
        return None;
    }
    let length = (written as usize).min(ROOM);
    let kept = &buffer[..length];
    // (it may count its terminating nothing; the path is what comes before it)
    let text = kept.split(|b| *b == 0).next().unwrap_or(kept);
    let dir = std::str::from_utf8(text).ok().filter(|d| !d.is_empty())?;
    Some(PathBuf::from(dir))
}
