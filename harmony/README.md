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
   - `files::still_writing()` 的 250 ms 重排 —— 保存/导出是后台线程，写盘期间不再排帧就会停在半路。
   - `startup::gpu_shown()` / `canvas_awaited()` / `canvas_shown()` —— 首帧与"画布是否真的画出来了"的记录，
     缺了会让后端信任状态与"正在准备"提示永不前进。

9. **`egui::Event::ModifiersChanged` 只在变化时推。**
   每帧都推会让 egui 认为"输入一直在变"，`repaint_delay` 恒为 0，CPU 从 3% 涨到 55%。
   同理，`app.input.commands` 非空时必须排一帧，否则落在画布上的笔画不会重绘（只有触摸移动会触发重绘）。

10. **GLES 下管线线程必须在首帧之后才起，且 `gl_busy()` 不能恒为 false。**
    wgpu-hal 的 GLES 后端抢上下文锁用 `try_lock_for(6s).expect(...)`——两个线程同时进就是
    `cdylib` 直接 abort，日志里什么都没有。桌面版的写法是：`is_gl()` 时把 waker 存进
    `canvases_later`，在 `draw()` 末尾已经 present 之后才 spawn；`gl_busy()` 报告线程是否仍在跑，
    `App::frame` 据此让出 100 ms。

11. **窗口的生命周期要自己抓住。**
    `OnSurfaceDestroyed` 一返回，系统就会销毁 `OHNativeWindow`，而应用线程可能正画着那一帧。
    所以 `procraft_start` 里用 `OH_NativeWindow_NativeObjectReference` 持住窗口，
    应用线程结束时才 `Unreference`（见 `ohos_window.rs` 的 `hold`/`let_go`）。

12. **`procraft_start` 在 ArkUI 的 UI 线程上跑，而提问的答复也只从那个线程来。**
    所以它**不能** `join` 上一个应用线程：那个线程可能正卡在 `platform::ask` 里等 UI 线程答复，
    互等就是最长十分钟的假死（转屏时最容易遇到）。改成把旧 handle 交给新线程，由新线程先 join
    （`stop_before`）。

13. **中英双语：翻译放在排版那一层，不放在每个调用点。**
    `look.rs` 的 `tracked_runs`/`wrapped`/`text_width`/`laid` 全都走 `lang::tr_cow`，
    于是"量多宽"和"画什么"永远是同一个字符串，不会因为翻译而对不齐。
    新加界面文字只要把英文放进 `i18n/strings.json` 与某个 `part*.zh.txt`，
    再 `python3 i18n/build_table.py` 重新生成 `desktop/src/lang/zh.rs` 即可；
    表里没有的字符串自动回退英文（egui 的 `Id`、ffmpeg 参数、格式缩写等**故意**不翻）。
    注意大小写转换要在翻译**之后**做（`tr(heading).to_uppercase()`），否则 `heading` 先变大写就查不到了。

14. **ArkTS 回调的 error 参数在成功时可能是 `undefined`，也可能是 `{code: 0}`。**
    写 `if (error)` 会误判成功为失败；写 `if (error.code)` 会在 `undefined` 上取属性而抛
    `TypeError`——ArkTS 里未捕获的异常直接杀进程（`Kill Reason: Js Error`）。
    正确写法是 `if (error && error.code)`。

15. **文件选择器给的是 URI，不是路径。**
    `picker.DocumentViewPicker.select()/save()` 返回 `file://docs/...`，直接当路径 `std::fs::read`
    会 ENOENT，而当路径写文件会真的在沙箱里建一个名叫 `file:` 的文件。
    约定是：Rust 只认路径；ArkTS 侧把选中的 URI `fs.copyFileSync` 到沙箱 `picked/`，
    保存则先给一个沙箱 `outbox/` 路径、由 250 ms 的 outbox 定时器在文件写完（`.part` 改名之后）再拷回去。
    文件夹选择同理（`DocumentSelectMode.FOLDER`，回写时逐个 `listFileSync` 拷贝）。

16. **提问是应用"推"给 shell 的，不是 shell 轮询。**
    起初 ArkTS 用 `setInterval(pump, 16)` 每帧问一次 `procraft.asks()`，实测空闲时就吃掉
    4.9% 的一个核——问了一辈子"没有"。现在 Rust 在入队时置一个 `AtomicBool` 并通过
    `procraft_set_ask_wake` 注册进来的回调唤醒页面（`napi_create_threadsafe_function`，
    跨线程安全），页面不再轮询；空闲降到 **0.2–0.45%**。
    `outbox/` 的拷贝仍用 250 ms 定时器，因为"沙箱里多了个文件"不是提问，没人会通知页面。

17. **`compatibleSdkVersion` 要写真实的最低 API。**
    应用靠 `OH_AbilityRuntime_ApplicationContextGetFilesDir`（API 16 起）找自己的目录，
    声明成 API 12 就是在承诺一个跑起来找不到自己文件的包。写成 `5.0.4(16)`——
    平台号与 API 号必须成对（hvigor 用 `hos-config.json` 的 `osVersionMapper` 校验，
    16 对应 `5.0.4`，不是 `5.0.1`），打包出来的 `minAPIVersion` 也就从 50000012 变成 50004016。

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
- **中文界面**：图库、Actions 面板六个分页、设置页（含分组标题）、关于页、警告框、笔刷库全部中文；
  设置 → 语言 可中英切换，整页即时重绘，选择写进 prefs 重启后保持
- **调整菜单 / 快捷菜单 / 手势说明** 全部中文（这些文字由 `crates/core` 的数据表给出，
  最初扫描只扫了 `desktop/src`，所以漏了 110 条；现在都在表里，排版层自动翻译，调用点一处未改）
- 空闲 CPU **0.2–0.45%**（曾因每帧推 `ModifiersChanged` 跑到 55%，又因 16 ms 轮询提问占 4.9%）
- **返回键**逐层退出：先收面板，再退出画布，最后退图库的栈（对应 Android 版的 `BackHandler`）
- **软键盘**在点画布标题、图层重命名等文本框时拉起，输入法组字正常
- **导出**保存到用户目录后可被系统选择器看到（`Untitled Artwork.png`，48 KB）
- **调色板名称与颜色名**中文（基础 / 大地，以及 灰红 / 深红 / 淡红 这类 73 个色名）

---

## 已知限制

- 打包用的签名 profile 绑定自己的 bundle 名，而 `AppScope/app.json5` 里用的是应用本身的 `io.github.artcraftandroid.procraft`；本地签名时把这个名字改成 profile 里的那一个即可，提交时不要带上改过的名字。
- 文件选择/剪贴板经 ArkTS shell 往返，首次拉起系统选择器有系统本身的延迟。
- 剪贴板**读**需要 `ohos.permission.READ_PASTEBOARD`，该权限是 `system_basic`，
  必须在签名 profile 里加 ACL 才授予；只改 `module.json5` 不会生效。目前读剪贴板会被
  pasteboard_service 明确拒绝（日志里有 `perm=ohos.permission.READ_PASTEBOARD`）。
  写剪贴板（文字与图片）不需要权限，已可用。
- 软键盘由 ArkTS 侧隐藏的 `TextInput` 承接焦点，输入法组字（CJK）经 `onDidInsert`/`onDidDelete`
  逐字送到应用；第三方输入法的候选词行为依赖系统，未在多种输入法上逐一验证。
- 手写笔的真实压感、倾斜已从 `OH_NativeXComponent` 取到并传入笔刷引擎，但尚未在真机手写笔上做压感曲线标定。
- 剪贴板**写图片**的链路（Rust 写 PNG → ArkTS 转 `PixelMap` → `pasteboard`）已经实现并有编译保证，
  但上游界面本身没有任何地方会发出"复制图片"命令（`egui::OutputCommand::CopyImage` 在 `desktop/` 与
  `crates/` 里只有消费者，没有生产者），所以在界面上点不到它。这是上游行为，不是移植缺失。
- 启动时笔刷形状与颗粒要生成 91 张 256×256 图（约 5.8 MB 像素），每张已按行分带并行到 8 个核；
  原先形状与颗粒是先后生成的，改成两半同时生成后实测从 533–674 ms 降到 **355–384 ms**。
  管线建立（3.6–4.4 s）仍发生在首帧之后，不挡界面。
