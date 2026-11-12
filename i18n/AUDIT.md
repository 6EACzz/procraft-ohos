# 移植层审计发现（第一轮，自行复核）

对照参考实现 `desktop/src/main.rs` + `screen.rs`（桌面版）逐项比对移植层 `ohos_main.rs` 等。

## 已确认并修复

1. **surface 销毁后应用永久死亡**（严重）
   `procraft_surface_destroyed` → `Said::Stop` → 循环 `break`，线程结束，但 `SAID`/`WAKE` 未清、
   无重启路径。XComponent 旋转/改尺寸/切前台都会重建 surface，之后 `procraft_start` 要么不调用，
   要么调用了也是第二个循环抢同一个 surface。
   修：新增 `RUNNING` 保存线程句柄；`stop_app()` 置 stopped → wake → join → 清全局；
   `procraft_start` 开头先 `stop_app()`。`TabletLoop::stop()`。

2. **`files::tell_what_was_told()` 从未调用**（高）
   桌面版在 `about_to_wait` 每轮调用，把写文件线程攒下的错误/提示弹出来。移植层完全没有这条线，
   所以"保存失败""导出失败"这类消息永远不显示，用户只看到操作没反应。
   待修：循环里补调用 + `still_writing()` 时限帧。

3. **GPU 设备丢失无处理**（高）
   桌面版 `gpu_lost()`：保存会话、丢 screen、`startup::start_again`、退出重来。移植层没有，
   设备丢失后只会一直画不出东西。
   待修：循环里检测 `gpu.is_lost()`，走对应处理（移植层没有"重开进程"的能力，改为提示 + 重建 surface/device）。

4. **密度变化没同步 canvas**（中）
   桌面版 `ScaleFactorChanged`：`session.set_display_scale(scale)` + `ui.place_canvas(...)`。
   移植层 `Said::Scale` 只改了 `input.density` 和 `screen.set_pixels_per_point`，
   session 的 `display_scale` 停留在开画布时的值 → 笔刷尺寸/命中判定在密度变化后失准。
   待修：Scale 分支补 `set_display_scale` + `place_canvas`。

5. **画布上 Ctrl+X/C/V 被 egui 截走**（中）
   桌面版每帧 `set_clipboard_shortcuts(!canvas_keys)`：画布打开且不要在文本框里时关掉 egui 的
   剪贴板快捷键，让快捷键走应用自己的层剪切/复制/粘贴。移植层从不设置，egui 默认 `true`，
   于是 Ctrl+C 变成 egui 的复制事件，应用的层复制收不到。
   待修：`ohos_screen` 增加同样的开关，每帧按状态设置。

6. **系统字体路径错误导致无 CJK**（高，已修）
   `system_fonts()` 在 unix 分支读 `/usr/share/fonts`，HarmonyOS 上没有这个路径 → 全是空，
   egui 自带字体无中文字形。
   修：新增 `#[cfg(target_env = "ohos")]` 分支，读 `/system/fonts`：
   `HarmonyOS_Sans_SC.ttf`（系统自己的中文面，最贴系统观感）、`NotoSansCJK-Regular.ttc`、
   Arab/Hebrew/Devanagari/Thai 各面。
   已验证：应用进程可读 `/system/fonts`（shell 不行），实测中文渲染正常。

7. **字体加载无幂等保护**（中）
   `install_fonts` 被十余处调用，每次 `set_fonts` + 起线程；OHOS 上每次读 40MB 字体。
   修：用 `ctx.data_mut` 的 temp 标记做 once。

## 待确认（等子代理审计交叉验证）

- `ask()` 里 `recv_timeout(600s)`：ArkTS 不应答就一直等，虽然超时兜底但 10 分钟等于挂死；
  且排队后没有任何机制通知 ArkTS 来取（只能靠它的 16ms 轮询），延迟不确定。
- `pen.rs` 的 `KINDS` HashMap 只增不删，长时间使用缓慢增长。
- `Said::Stop` 分支在主线程（NAPI 回调线程）里跑 `leave_artwork()`，会做完整保存，
  可能卡住 UI 线程；应移到 app 线程或异步。

## i18n 相关注意

- 所有文字最终走 `look::text*`（317 处）+ 26 个文件里的 `layout_no_wrap`，
  但 `layout_no_wrap` 既用于**绘制**也用于**量宽**，不能只在绘制处翻译，否则测量和绘制不一致。
  → 翻译必须在**调用点**做（传进 `text(...)` 之前），不能在 `look::text` 里做。
- `look::text` 不裁剪，靠 painter 的 clip_rect；中文通常更短，溢出风险低。
- 设置页 `settings.rs` 的 `Row::Toggle` 等是 `&'static str` 常量表，改成翻译需要返回 `&'static str`
  或 `String`；`tr()` 返回 `&'static str`，静态表可以直接用。
