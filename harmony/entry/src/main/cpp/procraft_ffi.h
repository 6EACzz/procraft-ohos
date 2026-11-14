#ifndef PROCRAFT_FFI_H
#define PROCRAFT_FFI_H

// What `libprocraft_ui.so` gives the shell (the app's own code, the desktop's interface built as
// a library). Every function here is exported by `procraft/desktop/src/ohos_main.rs`; this file
// was once a scaffold's guess at an interface that was never built that way, and the two do not
// agree unless they are kept together — so the declarations live here once, and `napi_init.cpp`
// takes them from here rather than writing its own copy that can drift.
//
// Every call is safe from the shell's own (ArkUI) thread: each one only takes a lock and wakes
// the app's thread, which does the work (`tell` in ohos_main.rs). The one exception is
// `procraft_start`, which waits for an app thread before it to finish.

#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

// The surface is ready: the app starts, drawing into `window` (the XComponent's own, taken as an
// ohos native window). `width` and `height` are the surface's pixels and `pixels_per_point` its
// density. Returns with the app started, not with it stopped: it runs on a thread of its own.
// Waits for a previous app thread to finish first (see `procraft_surface_destroyed`).
void procraft_start(void* window, uint32_t width, uint32_t height, float pixels_per_point);

// The folder the app keeps its own files in (its sandbox: nothing under $HOME is writable).
void procraft_own_folder(const uint8_t* bytes, size_t length);

// The surface is gone: the app is asked to stop (it is not waited for here).
void procraft_surface_destroyed();

// The app is to draw again (the shell's own side has news for it).
void procraft_wake();

// A touch or a pen came. `phase` is 0 (down), 1 (move), 2 (up) or 3 (cancel); `x` and `y` are
// the surface's pixels; `tilt` is radians from upright and `orientation` the direction it leans.
void procraft_touch(uint64_t id, uint32_t phase, double x, double y, double pressure, double tilt,
                    double orientation, bool pen, bool eraser);

// A mouse moved over the surface, with nothing held down.
void procraft_hover(uint64_t id, double x, double y, double tilt, double orientation, bool eraser);

// The mouse left the surface.
void procraft_hover_out(uint64_t id);

// A key came: `code` is the system's key code, `text` what it types (0 for none).
void procraft_key(uint32_t code, uint32_t text, bool pressed, bool repeat);

// What a keyboard typed, as UTF-8 (a soft keyboard's own characters arrive this way).
void procraft_text(const uint8_t* bytes, size_t length);

// The surface's size in pixels.
void procraft_resized(uint32_t width, uint32_t height);

// The page's density (its vp to its pixels).
void procraft_scale(float pixels_per_point);

// The panel's refresh rate, in Hz: the pace the app draws its frames at, since a frame a refresh at
// 120 Hz is twice the frames a second that 60 is. Nought (never sent) means 60.
void procraft_frame_rate(uint32_t hertz);

// The app's window has focus, or hasn't.
void procraft_focused(bool focused);

// A file was opened with the app (its path).
void procraft_dropped_file(const uint8_t* bytes, size_t length);

// What the app asked the shell for, since the last call: a JSON array of
// `{"id":N,"kind":"...","details":{...}}`, each answered by `procraft_answer`. The caller owns the
// string and hands it back with `procraft_asks_free`.
const char* procraft_asks();
void procraft_asks_free(const char* text);

// The shell's answer to one ask: the paths picked (one a line), the clipboard's contents, and
// whether the shell could do it at all. Either pointer may be null for none.
void procraft_answer(uint64_t id, const char* paths, const char* clipboard, bool done);

// Whether the app has something of its own to close before the shell closes the app (the page
// asks this when BACK is pressed, so BACK closes the app's own dialog or panel first).
bool procraft_can_go_back();

#ifdef __cplusplus
}
#endif

#endif // PROCRAFT_FFI_H
