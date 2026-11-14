# ProCraft → HarmonyOS：进度记录

（2026-11-14 14:35 更新。恢复时先读这份；两份审计报告在 `reports/`。）

## 一句话状态

**移植已完成并在真机验证可用。** 中英文双语完整（设置里可切换）、返回键逐层退出、
软键盘可拉起、文件选择/保存走系统选择器并双向暂存、空闲 CPU 0.2–0.45%。
`main` 是与上游差异最小化的稳定分支；性能优化单独在 `experimental/harmony-performance`。

## 代码位置

- 本地产出：`/static/Dev/Temp/procraft-harmony-20261110/`（`procraft/` 是镜像，无 .git）
- 真正仓库：构建机 `root@192.168.5.31:/root/procraft-harmony/procraft`
- 推送目标：`https://github.com/6EACzz/procraft-ohos`（凭据 `.secrets/github.txt`）
  - `main` = `9d40ff4`（稳定分支，已合并上游 `c330afd` = v0.0.6-beta）
  - `experimental/harmony-performance` = `82e57ed`（在 main 之上，性能 + 画笔纹理并行）
  - `wip/audit-fixes` = `ab2706e`（历史分支，内容已全部并入 main）
  - `backup/pre-upstream-merge` = `fbe49bf`（合并上游前的备份）
- 上游 `upstream/main` = `c330afd`（v0.0.6-beta），已合并进 `main`；两处冲突（`.gitignore`、`ui.rs` 的 `shortcuts`）都已手工解决，另修了三处上游改结构导致的不一致（`colour_models` 移入 `crates/core`、`stroke_sound`/`touch_keyboard` 模块、`export_plugin` 改走 `platform::pick`、`Action::PluginExport` 在 `app.rs` 补 arm）

## 交付物

`output/procraft-harmony-signed.hap`（43938254 B, md5 `7ba063f8…`）
`output/procraft-harmony-unsigned.hap`（43534103 B, md5 `f15749a0…`）
均已更新为合并上游后的构建（`0.0.5-beta` / minAPIVersion 50004016 / targetAPI 6.1.1(24)）。
CI 另发布 alpha：`harmony-v<版本>-alpha` 预发布，内含未签名 HAP + SHA256SUMS。

## 已完成并验证（HUAWEI MatePad 11.5''S，HarmonyOS 6.1.0 / API 24，2800×1840）

### 移植骨架
- 界面全部复用桌面版 egui 代码（约 4.3 万行），只换窗口/事件层：`ohos_window.rs`（XComponent 的
  `OHNativeWindow` → wgpu surface）+ `ohos_main.rs`（自有事件循环）+ `ohos_input.rs` + `ohos_screen.rs`
- GLES 后端（wgpu 的 Vulkan 后端不认 `OhosNdk` 句柄）；设备走 Maleoon 910 / OpenGL ES 3.2
- 沙箱目录用 `OH_AbilityRuntime_ApplicationContextGetFilesDir`（API 16+），不靠猜

### 存储 / 文件
- 自动保存到沙箱，重进应用作品仍在，点开能载入
- **导出**：系统选择器选路径，`.part` 写完改名后由 250 ms 定时器拷回用户目录；
  实测 `Untitled Artwork.png` 能在系统选择器里看到
- **导入**：`DocumentViewPicker` 拉取，`.procraft` 后缀过滤正常

### 返回键（对应 Android 版 `BackHandler`）
- 逐层退出：先收面板/菜单/文本框 → 再退全屏页面（Studio、裁剪、手势设置等）→ 再退画布 → 最后退图库的栈
- 靠 `ui.closes_something()`（`popover_open || !canvas_shown()`）+ `procraft_can_go_back` 实现

### 软键盘
- 点画布标题、图层重命名等文本框时拉起；`onDidInsert`/`onDidDelete` 逐字送到应用，
  中文输入法组字正常，退格正常

### 中英双语
- 翻译做在**排版层**（`look.rs` 的 `tracked_runs`/`wrapped`/`text_width`/`laid` 全走 `lang::tr_cow`），
  所以「量多宽」和「画什么」永远是同一串，不会因翻译而错位；~310 个绘制点一处未改
- 覆盖：图库、Actions 六个分页、设置页（含分组标题）、关于页、警告框、笔刷库、调整菜单、
  快捷菜单、手势说明、调色板名与 73 个颜色名
- 设置 → 帮助页「高级设置」→ Language 行可切换，整页即时重绘，写入 prefs 重启保持
- 表里没有的字符串自动回退英文（egui `Id`、ffmpeg 参数、格式缩写、笔刷名、作品名等**故意**不翻）
- `desktop/src/lang/zh.rs` 表 **1109 条**；重建：`python3 i18n/build_table.py`
- **双指轻点撤销 / 三指重做**已验证（`crates/core/src/touch.rs` 的 `fingers_did_tap`；实测加一层后
  双指轻点，Layer 2 被撤销掉、重做按钮变可用）。注意用 `uinput -T -m` 模拟时要传**成对的起点终点**
  （`-m x1 y1 x2 y2 x1 y1 x2 y2 <ms>`），否则手指移动了就不是 tap，什么都不会发生
- 图层 / 颜色 / 新建画布 / 设置 / 帮助页全部中文（`Layer 1` 这类是用户内容，不翻）
- `cargo clippy --release --lib --target aarch64-unknown-linux-ohos` **零警告零错误**
  （工作区开着 `clippy::indexing-slicing` 与 `clippy::unsafe-op-in-unsafe-fn`）

### 性能
| 项目 | 之前 | 现在 |
|---|---|---|
| 空闲 CPU（图库静止） | 55%（每帧推 `ModifiersChanged`）→ 3.7% → 4.9%（16ms 轮询提问） | **0.2–0.45%** |
| 笔刷形状+颗粒生成 | 533–674 ms | **355–384 ms** |
| 画布管线建立 | 3.6–4.4 s（首帧之后后台） | 同（不挡界面） |

`cargo clippy --release --lib --target aarch64-unknown-linux-ohos` **零警告零错误**（工作区开着
`clippy::indexing-slicing` 与 `clippy::unsafe-op-in-unsafe-fn`）。

空闲 CPU 的根因是 ArkTS 用 `setInterval(pump, 16)` 每帧问一次 `procraft.asks()`；
现在改成 Rust 入队时通过 `napi_threadsafe_function` 唤醒页面（`procraft_set_ask_wake`），页面不再轮询。

### 健壮性（两份审计报告的主要修复）
- R1 `procraft_start` 在 shell UI 线程 join 上一条应用线程 → 最长 600s 死锁；改成交给新线程 join
- R2 surface 销毁后渲染线程仍可能画进去 → 用 `OH_NativeWindow_NativeObjectReference` 持住窗口
- R3 GLES 下管线线程与渲染线程抢 GL 上下文（wgpu-hal 6 秒超时直接 abort）→ 改回桌面端做法：
  GL 下由首帧后的 `draw` 启动，`gl_busy()` 真实转发
- R8 `PlatformOutput` 被丢弃 → 处理 `CopyText`/`CopyImage`/`OpenUrl`，并由 `ime.is_some()` 驱动软键盘
- R9 循环不为进行中的工作重新排帧 → 补 `files::still_writing()` 的 250ms 重排 + `startup::gpu_shown()` 等
- R31 `predicted_dt: 0.0` 让所有动画停住 → 改回 `1.0/60.0`
- A1/A3 协议：剪贴板图片改为 Rust 写 PNG 发路径，ArkTS 读 PNG→PixelMap→pasteboard；
  选择器返回 URI，ArkTS 把 URI 拷进沙箱 `picked/`、把沙箱 `outbox/` 拷回 URI
- A2 `pendingPick`/`pendingClipboard` 单槽改按 ask id 的 `Map`
- A5 `onChange` → `onDidInsert`/`onDidDelete`；`onSubmit` 的 key code 13 → 2054
- A7 `onBackground`/`onForeground` 转发 `procraft.focused(false/true)`（曾隐藏时白烧 6.5% CPU）
- A9 `if (error.code)` → `if (error && error.code)`（成功时是 `{code:0}` 或 `undefined`，都会误判）
- A15 BACK 接 `procraft_can_go_back`
- A17 `compatibleSdkVersion` 从 `5.0.0(12)` 改 `5.0.4(16)`（平台号/API 号必须成对）
- 软键盘 150001：`.focusable(this.typing)` 依赖重绘时机 → 改为恒 `focusable(true)` 并立即请求焦点
- 图库「正在准备画布」不消失：`canvases_made` 后只 wake 不重绘 → 补 `app.again`
- 版本号对齐：`app.json5` 的 `0.1.0`/1000000 → `0.0.5-beta`/501

## 已知限制（不是 bug，或需要签名配置）

1. **剪贴板读** 需要 `ohos.permission.READ_PASTEBOARD`（`system_basic`），必须在签名 profile 里加 ACL；
   只改 `module.json5` 不生效。当前读剪贴板被 pasteboard_service 明确拒绝。写剪贴板不需要权限，可用。
2. **剪贴板写图片** 的链路已实现并有编译保证，但上游界面本身没有任何地方发出
   `egui::OutputCommand::CopyImage`（只有消费者、没有生产者），所以界面上点不到。这是上游行为。
3. **bundle 名**：仓库里是 `io.github.artcraftandroid.procraft`（应用自己的名字）。本机签名
   profile 绑的是旧的调试名 `io.github.otd.harmonyTest`，所以本地装机验证要用
   `/root/otd-keys/test-build.sh`（只在构建期替换包名、结束还原，不入库）。
4. 手写笔压感/倾斜已接入笔刷引擎，但未在真机手写笔上做压感曲线标定。
5. 笔刷名与作品名不翻译（是用户内容）。
6. 空闲 60 fps 锁帧可能是 HarmonyOS 自身行为（用户提示）。

## 关键环境（别重复摸索）

- 构建机：`root@192.168.5.31`，用 `tools/sshrun.sh`；`cargo` 要 `export PATH=$HOME/.cargo/bin:$PATH`
- 同步文件：`tools/put.sh desktop/src/x.rs`（**不带** `procraft/` 前缀）
- 构建：`bash harmony/build.sh`；签名 `/root/otd-keys/sign.sh <in> <out>`
- 设备：`192.168.5.246:40703`，`tools/dev.sh {sh|shot|tap|start|stop|log|fetch}`
- 截图回读 2526×1660，`uinput` 用设备像素 2800×1840，**坐标乘 1.1086**
- 触摸画布是取色不是画画（设计如此）；`uinput -T -g` 需要 press≥500ms
- `param set hilog.loggable.global I` 才看得到 `I` 级日志
- 设备锁屏 shell 解不开（需人脸/密码），用 `power-shell timeout -o 1800000` + `power-shell wakeup`
- 提交信息：本地写文件 → `base64 -w0` → `tools/sshrun.sh "echo $b64|base64 -d > .../m/msg.txt && git commit -F"`
