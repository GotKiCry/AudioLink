# Android UI 重构（2026-09-20）

依据：根目录 `DESIGN.md`，沿用 Fluent 2 蓝灰令牌、4dp 控件 / 8dp 卡片、原生 Compose 交互。

## 第二轮：强化 Fluent 表达

针对第一轮仍像默认 Compose 卡片表单的问题，保留 Fluent 2，并把视觉细节下沉到共享组件：

- `Fluent.kt` 集中实体表面的细描边、明暗内边、中性控件填充和下沉输入槽；无需壁纸权限或实时模糊。
- `FluentTextField` 使用原生文本编辑能力、外置标签和强调底边，连接主按钮按内容宽度靠右。
- 轻量顶部导航以短选中指示替代大块蓝色填充；页面标题使用 28/36 字阶。
- `AppearancePicker` 用实际深浅色板提供外观缩略图；`ConnectionGuide` 显示音频方向。
- 本地截图位于 `.tmp-shots/android-fluent/`，临时检查脚本为 `.tmp-shots/android_fluent_smoke.py`；这些排查产物由 `.gitignore` 排除，不属于仓库交付物。

第二轮已重新通过双 ABI Debug / Release 构建、237 项 JVM 单测与 Lint（0 错误）。真机复查覆盖新输入框的聚焦 / 输入 / 键盘、收发切换、设置子页返回、表单保留、深浅主题，以及 360dp 的 1.3× 字体和 840dp 宽屏。本地比较图位于 `.tmp-shots/android-fluent/comparison.png`，不随代码提交。

英文接收页已单独复查。验证后清空了测试地址，并恢复手机分辨率、密度、字号、自动旋转和应用语言；Debug 包与原有 Release 包并存。

## 改动

- 单页控制台提供接收 / 发送视图，默认接收；切换视图不改变音频服务或采集源。
- 未连接时显示连接表单与准备说明；配对时只强调输入 PIN；连接后优先显示播放状态、音量和断开。新增连接可展开，每台设备保留音量、静音、断开，地址与信任信息按需展开。
- 发送视图直接提供地址、等待接入、采集源和发送控制；开始 / 停止按发送状态呈现。
- 设置集中主题、后台运行、更新、许可和折叠诊断。设置和子页逐级返回，表单与滚动状态保留。
- 修正 M3 默认字号、旧丝印大写与字距；共享表单宽度、滑块、语义标题和 48dp 触控区域。
- 中英文新增文案及用户手册同步更新。

## 验证

- `pwsh android/scripts/build-rust.ps1 -SkipUniffi`：arm64-v8a / armeabi-v7a 原生库构建通过，保留现有 Kotlin 绑定。
- `pwsh tools/gradlew.ps1 -JavaHome <本机 JDK 25> assembleDebug assembleRelease testDebugUnitTest lintDebug`：通过，双 ABI Debug / Release APK 已生成。
- JVM 单测：237 项，0 失败、0 错误、0 跳过。
- Android Lint：0 错误、12 告警、1 提示；告警涉及现有 SDK 目标、依赖、备份、未用资源及 KTX 建议，不修改这些非 UI 配置。
- 真机 PHK110，Debug 包：接收 / 发送切换、设置 / 更新 / 许可返回、输入地址保留、键盘布局、深浅主题、英文接收页均已验证。
- 真机临时配置：360dp 左右宽度、1.3× 字体；840dp 宽度。截图检查完成，原分辨率、密度、字体缩放和语言均已恢复。
- 本轮 UI 与文档 `git diff --check` 通过；仓库整体仍有既有生成绑定的尾随空格。
- 完整代码提交检查：Rust 格式与双端 Clippy 通过；内核 541 项测试通过、1 项跳过；桌面 Rust 49 项通过、1 项硬件测试忽略；桌面前端构建与 157 项测试通过。重新生成的 Kotlin FFI 绑定与当前源码逐字节一致，双 ABI 库与 APK 均包含全部 18 个 checksum 符号。

本机截图：`.tmp-shots/android-refactor/`；交互验证脚本：`.tmp-shots/android_ui_smoke.py`。

## 未覆盖

- 本次真机检查未重新建立音频会话，未完成实时配对、多路混音、断网重连与后台长时播放的端到端验收。
- TalkBack 人工朗读与完整英文设置流程未验；省电引导仍沿用服务层已有中文文案。
