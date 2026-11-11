#ifndef PROCRAFT_FFI_H
#define PROCRAFT_FFI_H

#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

// Core lifecycle and window attachment
void procraft_init(const char* files_dir);
uint64_t procraft_acquire_native_window(void* window);
void procraft_release_native_window(uint64_t id);

// Session management
void* procraft_session_create(uint32_t width, uint32_t height);
void procraft_session_destroy(void* session);
bool procraft_session_attach_window(void* session, uint64_t window_id, uint32_t width, uint32_t height);
void procraft_session_resize_window(void* session, uint32_t width, uint32_t height);
void procraft_session_detach_window(void* session);

// Touch and pen inputs
void procraft_session_touch_down(
    void* session,
    uint64_t id,
    float x,
    float y,
    float pressure,
    float tilt,
    float orientation,
    bool is_pen
);

void procraft_session_touch_move(
    void* session,
    uint64_t id,
    float x,
    float y,
    float pressure,
    float tilt,
    float orientation
);

void procraft_session_touch_up(
    void* session,
    uint64_t id,
    float x,
    float y,
    float pressure,
    float tilt,
    float orientation
);

void procraft_session_touch_cancel(void* session);

// View gestures
void procraft_session_view_gesture(
    void* session,
    float pivot_x,
    float pivot_y,
    float pan_x,
    float pan_y,
    float scale,
    float rotation
);
void procraft_session_fit_view(void* session);

// Painting commands
bool procraft_session_undo(void* session);
bool procraft_session_redo(void* session);
void procraft_session_clear_layer(void* session);
void procraft_session_set_tool(void* session, int32_t tool);
void procraft_session_set_brush_size(void* session, float slider);
void procraft_session_set_brush_opacity(void* session, float slider);
void procraft_session_set_color(void* session, float r, float g, float b);
void procraft_session_get_color(void* session, float* out_rgb);
bool procraft_session_save_document(void* session, const char* path);

#ifdef __cplusplus
}
#endif

#endif // PROCRAFT_FFI_H
