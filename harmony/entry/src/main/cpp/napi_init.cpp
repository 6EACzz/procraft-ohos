// The tablet's own half: the XComponent's surface and what the system gives the app (touches,
// a pen, keys, its size), handed to the app's own code (`procraft_ui`, the desktop's interface
// built as a library), and the shell's answers to what that asks (a file picked, a message shown,
// the clipboard read and written, the soft keyboard).
//
// The app itself runs on a thread of its own (`procraft_start`); this only passes on what came.

#include <napi/native_api.h>
#include <hilog/log.h>
#include <ace/xcomponent/native_interface_xcomponent.h>
#include <cstdint>
#include <cstring>
#include <cmath>
#include <string>
#include <vector>

#undef LOG_DOMAIN
#undef LOG_TAG
#define LOG_DOMAIN 0x3200
#define LOG_TAG "ProCraft"

// The app's own code (the desktop's interface, built as a library: `libprocraft_ui.so`) — the
// declarations of what it gives us live in one place, so this file and the app's side of them
// cannot drift apart.
#include "procraft_ffi.h"

namespace {

// The XComponent's window, once it exists.
OH_NativeXComponent* g_component = nullptr;
void* g_window = nullptr;
uint32_t g_width = 0;
uint32_t g_height = 0;
// The page's density (its vp to its pixels). 2.0f is only a guess for the frames before the page
// has been laid out: it sends the real one as soon as it has (`setScale`), and this keeps it so
// that a start coming after that measurement starts at the right density (a start coming before
// it is put right by the `procraft_scale` that `setScale` sends, once the app is up).
float g_scale = 2.0f;

// The page's own measurement of the surface, in pixels (what `resized` was last given by
// `onAreaChange`). The component's size is documented in vp, so it is not the same number; once
// the page has said, what it said is the surface's real size and the component's own report must
// not overwrite it with a value in another unit.
bool g_size_from_page = false;
uint32_t g_page_width = 0;
uint32_t g_page_height = 0;

// A JS string out of a N-API value, as UTF-8.
std::string StringOf(napi_env env, napi_value value) {
    size_t length = 0;
    if (napi_get_value_string_utf8(env, value, nullptr, 0, &length) != napi_ok) return {};
    // `napi_get_value_string_utf8` writes the string *and* the NUL that ends it, so the buffer
    // has to hold one more byte than the length it reported (asking for `length + 1` into a
    // `length`-sized string writes one byte past its end).
    std::string text(length + 1, '\0');
    napi_get_value_string_utf8(env, value, text.data(), text.size(), &length);
    text.resize(length);
    return text;
}

// A N-API number, or `fallback`.
double DoubleOf(napi_env env, napi_value value, double fallback) {
    double number = fallback;
    if (napi_get_value_double(env, value, &number) != napi_ok) return fallback;
    return number;
}

// A N-API boolean, or false.
bool BoolOf(napi_env env, napi_value value) {
    bool flag = false;
    napi_get_value_bool(env, value, &flag);
    return flag;
}

// The surface is ready: the app starts drawing in it.
void OnSurfaceCreated(OH_NativeXComponent* component, void* window) {
    uint64_t width = 0;
    uint64_t height = 0;
    OH_NativeXComponent_GetXComponentSize(component, window, &width, &height);
    g_component = component;
    g_window = window;
    if (g_size_from_page) {
        // the page has already measured the surface in its own pixels (this is a surface coming
        // back — the panel turned, or the page was rebuilt): its measurement is the surface's
        // real size, and the component's own report here is documented in vp
        g_width = g_page_width;
        g_height = g_page_height;
    } else {
        g_width = static_cast<uint32_t>(width);
        g_height = static_cast<uint32_t>(height);
    }
    OH_LOG_INFO(LOG_APP, "the surface is ready: %{public}ux%{public}u", g_width, g_height);
    // (this is only the size the app starts at: the page's own measurement in pixels comes through
    // `resized` as soon as it is laid out, and is the one the app resizes to)
    procraft_start(window, g_width, g_height, g_scale);
}

// The surface's size changed (the panel turned, or its window did).
void OnSurfaceChanged(OH_NativeXComponent* component, void* window) {
    uint64_t width = 0;
    uint64_t height = 0;
    OH_NativeXComponent_GetXComponentSize(component, window, &width, &height);
    if (width == 0 || height == 0) return;
    g_width = static_cast<uint32_t>(width);
    g_height = static_cast<uint32_t>(height);
    OH_LOG_INFO(LOG_APP, "the surface is now %{public}ux%{public}u", g_width, g_height);
    if (g_size_from_page) {
        // the page has measured the surface itself, in its pixels, and that measurement is the
        // one to keep: this one is documented in vp, and handing it over would resize the app's
        // drawing to a size in the other unit (the page's own `resized` follows this callback
        // anyway, and is what actually resizes)
        return;
    }
    procraft_resized(g_width, g_height);
}

// The surface is gone (the app is ending, or the page went away).
void OnSurfaceDestroyed(OH_NativeXComponent* component, void* window) {
    OH_LOG_INFO(LOG_APP, "the surface is gone");
    procraft_surface_destroyed();
    // both cleared: a touch or a key still in flight must not be passed on to an app that has been
    // asked to stop (the dispatch handlers below let nothing through while there is no window)
    g_window = nullptr;
    g_component = nullptr;
}

// A touch came: a finger's, or the pen's.
void OnTouch(OH_NativeXComponent* component, void* window) {
    // nothing to draw into, and nothing to draw with: the surface is on its way in or out
    if (g_window == nullptr) return;
    OH_NativeXComponent_TouchEvent event;
    if (OH_NativeXComponent_GetTouchEvent(component, window, &event) != OH_NATIVEXCOMPONENT_RESULT_SUCCESS) return;
    OH_LOG_INFO(LOG_APP, "a touch: %{public}u points", event.numPoints);
    for (uint32_t i = 0; i < event.numPoints; ++i) {
        const OH_NativeXComponent_TouchPoint& point = event.touchPoints[i];
        uint32_t phase = 1;
        switch (point.type) {
            case OH_NATIVEXCOMPONENT_DOWN:
                phase = 0;
                break;
            case OH_NATIVEXCOMPONENT_UP:
                phase = 2;
                break;
            case OH_NATIVEXCOMPONENT_CANCEL:
                phase = 3;
                break;
            default:
                phase = 1;
                break;
        }
        // The pen's own: its pressure, the way it leans, and which end touches.
        float tilt_x = 0.0f;
        float tilt_y = 0.0f;
        OH_NativeXComponent_GetTouchPointTiltX(component, i, &tilt_x);
        OH_NativeXComponent_GetTouchPointTiltY(component, i, &tilt_y);
        OH_NativeXComponent_TouchPointToolType tool = OH_NATIVEXCOMPONENT_TOOL_TYPE_UNKNOWN;
        OH_NativeXComponent_GetTouchPointToolType(component, i, &tool);
        bool pen = tool == OH_NATIVEXCOMPONENT_TOOL_TYPE_PEN || tool == OH_NATIVEXCOMPONENT_TOOL_TYPE_RUBBER ||
                   tool == OH_NATIVEXCOMPONENT_TOOL_TYPE_BRUSH;
        bool eraser = tool == OH_NATIVEXCOMPONENT_TOOL_TYPE_RUBBER;
        double pressure = point.force > 0.0f ? static_cast<double>(point.force) : 1.0;
        // Radians from upright, and the direction it leans.
        double tilt = std::sqrt(static_cast<double>(tilt_x) * tilt_x + static_cast<double>(tilt_y) * tilt_y) *
                      (3.14159265358979 / 180.0);
        double orientation = std::atan2(static_cast<double>(tilt_y), static_cast<double>(tilt_x));
        procraft_touch(static_cast<uint64_t>(point.id), phase, static_cast<double>(point.x),
                       static_cast<double>(point.y), pressure, tilt, orientation, pen, eraser);
    }
}

OH_NativeXComponent_Callback g_callback = {
    .OnSurfaceCreated = OnSurfaceCreated,
    .OnSurfaceChanged = OnSurfaceChanged,
    .OnSurfaceDestroyed = OnSurfaceDestroyed,
    .DispatchTouchEvent = OnTouch,
};

// A mouse's move over the surface, its buttons and its wheel: a desktop's own. The app takes it
// as a pointer that is down and up (the same id every time, since a mouse is one pointer), as a
// pen's touch is taken, so a mouse draws and pans exactly as a pen does.
constexpr uint64_t MOUSE_ID = 0xFFFF;

void OnMouse(OH_NativeXComponent* component, void* window) {
    if (g_window == nullptr) return;
    OH_NativeXComponent_MouseEvent event;
    if (OH_NativeXComponent_GetMouseEvent(component, window, &event) != OH_NATIVEXCOMPONENT_RESULT_SUCCESS) return;
    const double x = static_cast<double>(event.x);
    const double y = static_cast<double>(event.y);
    switch (event.action) {
        case OH_NATIVEXCOMPONENT_MOUSE_PRESS:
            procraft_touch(MOUSE_ID, 0, x, y, 1.0, 0.0, 0.0, false, false);
            break;
        case OH_NATIVEXCOMPONENT_MOUSE_RELEASE:
            procraft_touch(MOUSE_ID, 2, x, y, 1.0, 0.0, 0.0, false, false);
            break;
        case OH_NATIVEXCOMPONENT_MOUSE_CANCEL:
            procraft_touch(MOUSE_ID, 3, x, y, 1.0, 0.0, 0.0, false, false);
            break;
        default:
            // a move: down it paints, up it hovers (the brush's outline follows it)
            procraft_touch(MOUSE_ID, 1, x, y, 1.0, 0.0, 0.0, false, false);
            procraft_hover(MOUSE_ID, x, y, 0.0, 0.0, false);
            break;
    }
}

void OnHover(OH_NativeXComponent* component, bool isHover) {
    if (!isHover) {
        // the mouse left: the brush's outline goes
        procraft_hover_out(MOUSE_ID);
    }
}

OH_NativeXComponent_MouseEvent_Callback g_mouse_callback = {
    .DispatchMouseEvent = OnMouse,
    .DispatchHoverEvent = OnHover,
};

// A hardware key came (a keyboard attached, or the pen's own buttons).
//
// This can only say *which* key and whether it went down: the native key event hands over the
// code and the action and nothing else — there is no accessor on it for the character the key
// typed, nor for whether the system is repeating it (see `OH_NativeXComponent_KeyEvent` in
// `native_interface_xcomponent.h`). So `text` stays 0 and `repeat` false here.
//
// Nothing is lost by that for the ordinary keys: the app works the character of a letter, digit
// or punctuation mark out of the key's code itself (`key_of` in `ohos_main.rs`). A character no
// key code spells — a composed CJK glyph, an accent — is the one thing this cannot carry, and it
// comes from the page instead (`onKeyEvent` in `Index.ets` knows its `keyText` and passes the
// non-ASCII ones through `procraft.text`, which is also the way the soft keyboard's characters
// arrive).
void OnKey(OH_NativeXComponent* component, void* window) {
    OH_NativeXComponent_KeyEvent* event = nullptr;
    if (OH_NativeXComponent_GetKeyEvent(component, &event) != OH_NATIVEXCOMPONENT_RESULT_SUCCESS || event == nullptr) return;
    OH_NativeXComponent_KeyAction action;
    OH_NativeXComponent_GetKeyEventAction(event, &action);
    OH_NativeXComponent_KeyCode code;
    OH_NativeXComponent_GetKeyEventCode(event, &code);
    bool pressed = action == OH_NATIVEXCOMPONENT_KEY_ACTION_DOWN;
    // the key as the app's own (its own codes are those of the system's keys)
    procraft_key(static_cast<uint32_t>(code), 0, pressed, false);
}

// The folder the app keeps its own files in (a tablet's sandbox: nothing under $HOME is writable).
napi_value SetOwnFolder(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value argv[1] = {nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    if (argc < 1) return nullptr;
    std::string dir = StringOf(env, argv[0]);
    procraft_own_folder(reinterpret_cast<const uint8_t*>(dir.data()), dir.size());
    return nullptr;
}

// The interface's scale (the page's density: its points to pixels).
napi_value SetScale(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value argv[1] = {nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    if (argc >= 1) {
        g_scale = static_cast<float>(DoubleOf(env, argv[0], g_scale));
        procraft_scale(g_scale);
    }
    return nullptr;
}

// The page's size in pixels (the surface's own): the app draws at it.
napi_value Resized(napi_env env, napi_callback_info info) {
    size_t argc = 2;
    napi_value argv[2] = {nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    if (argc < 2) return nullptr;
    uint32_t width = 0;
    uint32_t height = 0;
    napi_get_value_uint32(env, argv[0], &width);
    napi_get_value_uint32(env, argv[1], &height);
    if (width == 0 || height == 0) return nullptr;
    g_width = width;
    g_height = height;
    // this side has measured the surface itself, so it is the one that counts (see
    // `OnSurfaceChanged`, which stops resizing to the component's own report once this is set)
    g_size_from_page = true;
    g_page_width = width;
    g_page_height = height;
    procraft_resized(width, height);
    return nullptr;
}

// What the app asked the shell for, since the last call: an array of `{id, kind, details}`, each
// answered by `answer(id, paths, clipboard, done)`. The shell calls this each frame.
napi_value Asks(napi_env env, napi_callback_info info) {
    // always a string: the page reads .length of what this gives, and an undefined or null there
    // is a TypeError that stops the process
    napi_value result = nullptr;
    if (napi_create_string_utf8(env, "[]", 2, &result) != napi_ok) {
        return nullptr;
    }
    const char* text = procraft_asks();
    if (text == nullptr) return result;
    napi_value said = nullptr;
    if (napi_create_string_utf8(env, text, NAPI_AUTO_LENGTH, &said) == napi_ok) {
        result = said;
    }
    procraft_asks_free(text);
    return result;
}

// The shell's answer to one ask: the paths picked (one a line), the clipboard's contents, and
// whether the shell could do it at all.
napi_value Answer(napi_env env, napi_callback_info info) {
    size_t argc = 4;
    napi_value argv[4] = {nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    if (argc < 1) return nullptr;
    int64_t id = 0;
    napi_get_value_int64(env, argv[0], &id);
    std::string paths = argc >= 2 ? StringOf(env, argv[1]) : std::string();
    std::string clipboard = argc >= 3 ? StringOf(env, argv[2]) : std::string();
    bool done = argc >= 4 ? BoolOf(env, argv[3]) : true;
    procraft_answer(static_cast<uint64_t>(id), paths.empty() ? nullptr : paths.c_str(),
                    clipboard.empty() ? nullptr : clipboard.c_str(), done);
    return nullptr;
}

// The shell tells the app it has focus, or hasn't.
napi_value Focused(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value argv[1] = {nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    procraft_focused(argc >= 1 ? BoolOf(env, argv[0]) : true);
    return nullptr;
}

// A hardware key came (a keyboard attached, or the pen's own buttons).
napi_value Key(napi_env env, napi_callback_info info) {
    size_t argc = 4;
    napi_value argv[4] = {nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    if (argc < 1) return nullptr;
    uint32_t code = 0;
    napi_get_value_uint32(env, argv[0], &code);
    uint32_t text = 0;
    if (argc >= 2) napi_get_value_uint32(env, argv[1], &text);
    bool pressed = argc >= 3 ? BoolOf(env, argv[2]) : true;
    bool repeat = argc >= 4 ? BoolOf(env, argv[3]) : false;
    procraft_key(code, text, pressed, repeat);
    return nullptr;
}

// What a soft keyboard typed.
napi_value Text(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value argv[1] = {nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    if (argc < 1) return nullptr;
    std::string text = StringOf(env, argv[0]);
    procraft_text(reinterpret_cast<const uint8_t*>(text.data()), text.size());
    return nullptr;
}

// A file opened with the app (its path).
napi_value DroppedFile(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value argv[1] = {nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    if (argc < 1) return nullptr;
    std::string path = StringOf(env, argv[0]);
    procraft_dropped_file(reinterpret_cast<const uint8_t*>(path.data()), path.size());
    return nullptr;
}

// The window is to be redrawn (a wake from the shell's own side).
napi_value Wake(napi_env env, napi_callback_info info) {
    procraft_wake();
    return nullptr;
}

// Whether the app has something of its own to close before the shell should close the app: the
// page asks this when BACK is pressed (see `onBackPress` in `Index.ets`), so BACK closes a dialog
// or a panel of the app's rather than leaving the whole app.
napi_value CanGoBack(napi_env env, napi_callback_info info) {
    napi_value result = nullptr;
    // (a boolean always: the page reads it as one, and an undefined is not)
    napi_get_boolean(env, procraft_can_go_back(), &result);
    return result;
}

} // namespace

EXTERN_C_START
static napi_value Init(napi_env env, napi_value exports) {
    napi_property_descriptor desc[] = {
        {"setOwnFolder", nullptr, SetOwnFolder, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"setScale", nullptr, SetScale, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"resized", nullptr, Resized, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"focused", nullptr, Focused, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"key", nullptr, Key, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"text", nullptr, Text, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"droppedFile", nullptr, DroppedFile, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"wake", nullptr, Wake, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"asks", nullptr, Asks, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"answer", nullptr, Answer, nullptr, nullptr, nullptr, napi_default, nullptr},
        {"canGoBack", nullptr, CanGoBack, nullptr, nullptr, nullptr, napi_default, nullptr},
    };
    napi_define_properties(env, exports, sizeof(desc) / sizeof(desc[0]), desc);

    // The XComponent's own surface, as it comes and goes.
    napi_value export_instance = nullptr;
    if (napi_get_named_property(env, exports, OH_NATIVE_XCOMPONENT_OBJ, &export_instance) == napi_ok) {
        OH_NativeXComponent* component = nullptr;
        if (napi_unwrap(env, export_instance, reinterpret_cast<void**>(&component)) == napi_ok && component != nullptr) {
            OH_NativeXComponent_RegisterCallback(component, &g_callback);
            OH_NativeXComponent_RegisterMouseEventCallback(component, &g_mouse_callback);
            OH_NativeXComponent_RegisterKeyEventCallback(component, OnKey);
            g_component = component;
            OH_LOG_INFO(LOG_APP, "the XComponent is ready");
        }
    }
    return exports;
}
EXTERN_C_END

static napi_module g_module = {
    .nm_version = 1,
    .nm_flags = 0,
    .nm_filename = nullptr,
    .nm_register_func = Init,
    .nm_modname = "procraft",
    .nm_priv = nullptr,
    .reserved = {0},
};

extern "C" __attribute__((constructor)) void RegisterProCraftModule() {
    napi_module_register(&g_module);
}
