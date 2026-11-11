# ProCraft for HarmonyOS

ProCraft 的 HarmonyOS（API 24+，HarmonyOS 6/7）版本。**界面、画布、笔刷引擎与桌面版完全同一份代码**，只替换了平台层。

---

## 这是怎么做到的

ProCraft 的桌面版本身就是 **Rust + egui + wgpu**：界面全部由 Rust 直接绘制到一块 wgpu 表面上，不依赖任何窗口工具包的控件。所以移植不是重写界面，而是**换一个窗口**：

| 层 | 桌面 | HarmonyOS（此处） |
|---|---|---|
| 界面（图库、画布工具栏、面板、Brush Studio、快捷键） | 同一份 `desktop/src/*.rs`（约 4.3 万行） | **完全相同** |
| 画布与笔刷渲染 | `crates/engine` + `crates/gpu` | **完全相同** |
| 会话与手势 | `crates/core` | **完全相同** |
| 窗口与事件循环 | winit | `ohos_window.rs`（XComponent 的 `OHNativeWindow`）+ `ohos_main.rs`（自有事件循环） |
| 输入 | winit 事件 | `ohos_input.rs`：把触摸/手写笔/鼠标转成同样的 `winit::event::WindowEvent` |
| 文件选择与剪贴板 | `rfd` / `arboard` | `platform.rs` → ArkTS shell（系统选择器与剪贴板） |

关键点：`App` 只通过两个 trait 与平台接触 ——
- `host.rs` 的 **`Host`**：窗口尺寸、缩放、GPU、egui 上下文、画一帧、画笔面板、画布管线状态；
- `app.rs` 的 **`Loop`**：何时唤醒、何时结束。

两个平台各实现一次，其余全部复用。

### 为什么用 OpenGL ES 而不是 Vulkan

wgpu 的 Vulkan 后端不认 `OhosNdk` 窗口句柄（`create_surface` 直接返回 "not a Vulkan-compatible handle"），只有 GLES/EGL 后端认得。因此 `crates/gpu` 在 `target_env = "ohos"` 时只启用 `wgpu::Backends::GL`。设备上实际走的是 **Maleoon 910 / OpenGL ES 3.2**。

### 几个必须踩对的细节

1. **应用沙箱目录只能用系统 API 问，不能猜。**
   `std::env::var_os("HOME")` 是 `/storage/Users/currentUser`（不可写），`/proc/self/exe` 是 `/system/bin/appspawn`——应用是 appspawn fork 出来的，exe 从来不是自己的。两条路都推不出沙箱。
   唯一可靠的是 Core File Kit 的 `OH_AbilityRuntime_ApplicationContextGetFilesDir`（API 16+，`libability_runtime.so`），返回 `/data/storage/el2/base/haps/entry/files`。见 `desktop/src/ohos_files.rs`。
   拿错目录的后果不是报错而是**崩溃**：保存失败 → 回到图库 → 进程被杀。

2. **egui 的密度必须走 `ViewportInfo.native_pixels_per_point`，不能走 `set_pixels_per_point`。**
   后者内部是 `set_zoom_factor(ppp / native_ppp)`，而 `native_ppp` 默认 1.0，于是 2 会变成 4，界面按 5000×5000 点布局，只画出左上角一角。

3. **触摸坐标有两套约定，别搞混。** winit 的 `Touch.location` 是**窗口像素**（画布的 view、笔刷、命中测试都用它），egui 的事件是**点**。所以 `ohos_input` 里同一个触摸要按两套分别换算。

4. **`over_interface` 必须判 `layer.order != Background`**，否则所有触摸都被当成"点在界面上"，一笔都画不出来。

5. **`libprocraft_ui.so` 必须写 SONAME**，否则链接器把构建机的绝对路径写进 `DT_NEEDED`，设备上必然加载失败。

6. **`OH_LOG_Print` 的签名是 `(type, level, domain, tag, fmt, ...)`**，参数顺序错了会静默无输出。
   而且 level 是 **数字越大越严重**：`LOG_DEBUG=3, LOG_INFO=4, LOG_WARN=5, LOG_ERROR=6, LOG_FATAL=7`。
   映射错了会让所有 `info!` 显示成 `W`，看起来"日志在正常工作"，实际全错位——排查时会被带偏很久。

7. **ArkTS 里 `ask.details` 已经是解析好的对象，不是字符串。**
   Rust 端把 details 作为 JSON 值嵌进外层 JSON，`JSON.parse(text)` 之后它就是个对象；再对它 `JSON.parse()` 会抛
   `Unexpected Text in JSON: Invalid Token`。而 ArkTS 里**未捕获的异常会直接杀掉进程**（日志里是 `Kill Reason: Js Error`）。
   这就是"点很多控件就闪退"的根因：pick / say / clipboard / keyboard 全走这条路。

8. **原生事件循环必须把桌面版在 `about_to_wait` 里做的事都补上**，少一样就是一个 bug：
   - `input.tick()` —— 触摸的定时等待（双指轻点撤销靠它）
   - `screen.canvases_made()` —— 管线建好后要收掉"正在准备"状态，否则图库永远在等
   - `take_dropped()/imported()/artwork_read()/photo_read()` —— 后台读完的作品要能进来，否则图库点开一直卡"Opening…"
   - 帧节流（`frame_gap`）—— 否则 egui 每帧都请求重绘，CPU 跑满。注意 `app.again` 要在读到时 `take()` 掉，
     留着不动会让每一轮都判定"要画"。

---

## 目录

```
desktop/src/
  app.rs          App<H: Host, L: Loop>：全部 UI 逻辑（与桌面共用）
  host.rs         Host trait：窗口/GPU/egui 抽象
  platform.rs     文件对话框与剪贴板抽象
  ohos_files.rs   应用沙箱目录（系统 API 问出来，唯一可靠的来源）
  ohos_window.rs  OHNativeWindow → wgpu surface
  ohos_screen.rs  表面配置、egui 上下文、egui-wgpu 渲染、逐帧绘制
  ohos_input.rs   触摸/笔/鼠标/键盘 → winit 事件 + egui 事件
  ohos_platform.rs 向 shell 排队提问并按 id 收答
  ohos_main.rs    自有事件循环、C ABI、hilog 日志
  main.rs         桌面入口（winit，未改）
harmony/          HAP 工程
  build.sh        一键构建（Rust 库 + HAP）
  entry/src/main/cpp/napi_init.cpp   XComponent 回调 + NAPI 导出
  entry/src/main/ets/                ArkTS 外壳（页面、选择器、剪贴板、键盘）
```

---

## 构建

```bash
cd harmony
./build.sh            # release
./build.sh debug      # debug
```

`build.sh` 会用 SDK 自带 clang 交叉编译 `procraft_ui`，strip 后放进 `entry/libs/arm64-v8a/`，再调 hvigor 打包。

需要：HarmonyOS command-line-tools（hvigorw、ohpm、SDK native）、Rust 的 `aarch64-unknown-linux-ohos` target。

---

## 已验证（HUAWEI MatePad 11.5''S，HarmonyOS 6.1.0 / OpenHarmony 6.1.1 / API 24）

- 图库：标题、Select / Import / Photo / + 全部渲染正确，全屏 2800×1840
- 新建画布面板：Screen Size / Square / 4K / A4 / 4×6 Photo / Paper / Comic，含色彩空间与尺寸
- 新建 1920×1080 画布，图层自动建立（Layer 1 + Background color）
- **绘画**：连续多笔，笔刷边缘与抗锯齿正常；落笔位置准确
- 撤销 / 重做（含双指轻点）
- 图层面板
- **保存与重开**：画布自动保存到沙箱，回图库能看到缩略图与尺寸，退出应用重进仍在，点开能正常载入
- **回到图库不崩溃**（曾是保存失败导致进程被杀）
- **系统文件选择器**：Import / Insert a file 能拉起系统 `DocumentViewPicker`，中文界面正常
- **Actions 面板 / Share / Prefs / Help / Cut / Copy / Copy canvas / Paste** 均不再闪退
- 全屏沉浸、隐藏系统栏、保持常亮

---

## 已知限制

- 打包用的签名 profile 绑定 `io.github.otd.harmonyTest`，因此 `AppScope/app.json5` 里暂时用这个 bundle 名；换成自己的调试证书后改回 `io.github.artcraftandroid.procraft` 即可。
- 文件选择/剪贴板经 ArkTS shell 往返，首次拉起系统选择器有系统本身的延迟。
- 软键盘的显示目前靠 ArkTS 侧一个隐藏 TextInput 承接焦点；手写笔的真实压感、倾斜已从 `OH_NativeXComponent` 取到并传入笔刷引擎，但尚未在真机手写笔上做压感曲线标定。
