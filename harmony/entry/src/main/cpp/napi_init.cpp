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

// The app's own code (the desktop's interface, built as a library: `libprocraft_ui.so`).
extern "C" {
void procraft_start(void* window, uint32_t width, uint32_t height, float pixels_per_point);
void procraft_own_folder(const uint8_t* bytes, size_t length);
void procraft_surface_destroyed();
void procraft_wake();
void procraft_touch(uint64_t id, uint32_t phase, double x, double y, double pressure, double tilt,
                    double orientation, bool pen, bool eraser);
void procraft_hover(uint64_t id, double x, double y, double tilt, double orientation, bool eraser);
void procraft_hover_out(uint64_t id);
void procraft_key(uint32_t code, uint32_t text, bool pressed, bool repeat);
void procraft_text(const uint8_t* bytes, size_t length);
void procraft_resized(uint32_t width, uint32_t height);
void procraft_scale(float pixels_per_point);
void procraft_focused(bool focused);
void procraft_dropped_file(const uint8_t* bytes, size_t length);
const char* procraft_asks();
void procraft_asks_free(const char* text);
void procraft_answer(uint64_t id, const char* paths, const char* clipboard, bool done);
}

namespace {

// The XComponent's window, once it exists.
OH_NativeXComponent* g_component = nullptr;
void* g_window = nullptr;
uint32_t g_width = 0;
uint32_t g_height = 0;
float g_scale = 2.0f;
// The shell's own callbacks (a file picked, a message seen), which it sets from ArkTS.
napi_env g_env = nullptr;
napi_ref g_ask_ref = nullptr;

// A JS string out of a N-API value, as UTF-8.
std::string StringOf(napi_env env, napi_value value) {
    size_t length = 0;
    if (napi_get_value_string_utf8(env, value, nullptr, 0, &length) != napi_ok) return {};
    std::string text(length, '\0');
    napi_get_value_string_utf8(env, value, text.data(), length + 1, &length);
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
    g_width = static_cast<uint32_t>(width);
    g_height = static_cast<uint32_t>(height);
    OH_LOG_INFO(LOG_APP, "the surface is ready: %{public}ux%{public}u", g_width, g_height);
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
    procraft_resized(g_width, g_height);
}

// The surface is gone (the app is ending, or the page went away).
void OnSurfaceDestroyed(OH_NativeXComponent* component, void* window) {
    OH_LOG_INFO(LOG_APP, "the surface is gone");
    procraft_surface_destroyed();
    g_window = nullptr;
}

// A touch came: a finger's, or the pen's.
void OnTouch(OH_NativeXComponent* component, void* window) {
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

// A mouse's or a pen's move over the surface, its buttons, and a wheel: a desktop's own.
void OnMouse(OH_NativeXComponent* component, void* window) {
    OH_NativeXComponent_MouseEvent event;
    if (OH_NativeXComponent_GetMouseEvent(component, window, &event) != OH_NATIVEXCOMPONENT_RESULT_SUCCESS) return;
    // a mouse hovers as a pen does (the brush's outline follows it)
    procraft_hover(0xFFFF, static_cast<double>(event.x), static_cast<double>(event.y), 0.0, 0.0, false);
}

void OnMouseHover(OH_NativeXComponent* component, void* window) {
    OnMouse(component, window);
}

void OnHover(OH_NativeXComponent* component, bool isHover) {
    if (!isHover) {
        // the mouse left: the brush's outline goes
        procraft_hover_out(0xFFFF);
    }
}

OH_NativeXComponent_MouseEvent_Callback g_mouse_callback = {
    .DispatchMouseEvent = OnMouse,
    .DispatchHoverEvent = OnHover,
};

// A hardware key came (a keyboard attached, or the pen's own buttons).
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

// The app's own asks, answered by the shell: `procraftAsk(kind, json)` returns a string (a path,
// JSON of paths, the clipboard's contents) or nothing.
napi_value CallShell(napi_env env, const char* kind, const std::string& payload) {
    if (g_ask_ref == nullptr) return nullptr;
    napi_value ask = nullptr;
    if (napi_get_reference_value(env, g_ask_ref, &ask) != napi_ok || ask == nullptr) return nullptr;
    napi_value global = nullptr;
    napi_get_global(env, &global);
    napi_value kind_value = nullptr;
    napi_create_string_utf8(env, kind, NAPI_AUTO_LENGTH, &kind_value);
    napi_value payload_value = nullptr;
    napi_create_string_utf8(env, payload.c_str(), payload.size(), &payload_value);
    napi_value argv[2] = {kind_value, payload_value};
    napi_value answer = nullptr;
    if (napi_call_function(env, global, ask, 2, argv, &answer) != napi_ok) return nullptr;
    return answer;
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

// The shell's way to answer: its own function, kept for the app's asks.
napi_value SetAsk(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value argv[1] = {nullptr};
    napi_get_cb_info(env, info, &argc, argv, nullptr, nullptr);
    if (argc < 1) return nullptr;
    if (g_ask_ref != nullptr) {
        napi_delete_reference(env, g_ask_ref);
        g_ask_ref = nullptr;
    }
    napi_create_reference(env, argv[0], 1, &g_ask_ref);
    g_env = env;
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
    procraft_resized(width, height);
    return nullptr;
}

// What the app asked the shell for, since the last call: an array of `{id, kind, details}`, each
// answered by `answer(id, paths, clipboard, done)`. The shell calls this each frame.
napi_value Asks(napi_env env, napi_callback_info info) {
    const char* text = procraft_asks();
    if (text == nullptr) return nullptr;
    napi_value result = nullptr;
    napi_create_string_utf8(env, text, NAPI_AUTO_LENGTH, &result);
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

} // namespace

EXTERN_C_START
static napi_value Init(napi_env env, napi_value exports) {
    napi_property_descriptor desc[] = {
        {"setAsk", nullptr, SetAsk, nullptr, nullptr, nullptr, napi_default, nullptr},
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
