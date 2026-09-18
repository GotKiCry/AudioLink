# Microsoft Fluent 2 设计语言规范（Windows 11 口径）

> 适用对象：AudioLink 桌面端（Tauri 2 + React + Tailwind v4，Windows 11）。
> 抓取日期：2026-09-18。所有数值均来自下方「来源清单」中的一手文档或官方主题资源源码，
> 凡属推导或未找到权威值的条目，均在原处显式标注。
> 术语与 token 名保留英文，说明用中文。

---

## 0. 阅读前必须知道的三件事

### 0.1 本规范同时覆盖两套并行的 token 体系

Fluent 2 是**跨平台设计语言**，Windows 11 是它在 Windows 上的**原生实现**。两者 token 名不同、数值口径不同，不能混用：

| 体系 | 使用者 | 颜色 token 命名 | 圆角 token | 材质 |
| --- | --- | --- | --- | --- |
| **Fluent 2（Web）** | Fluent UI React v9、网页、跨平台 | `colorNeutralBackground1`、`colorBrandBackground` | `borderRadiusMedium = 4px` | 无（网页不做 Mica） |
| **Windows 11 / WinUI 3** | Windows 原生应用 | `SolidBackgroundFillColorBase`、`AccentFillColorDefault` | `ControlCornerRadius = 4,4,4,4`、`OverlayCornerRadius = 8,8,8,8` | Mica / Mica Alt / Acrylic / Smoke |

AudioLink 是 Tauri 桌面应用：**视觉效果照 Windows 11/WinUI 3 口径实现**（因为要和系统窗口、Mica、圆角一致），
**CSS 变量命名照 Fluent 2 Web 口径**（因为 React 生态和 Fluent UI 组件库用这套名字）。本文给出两套的映射表。

### 0.2 WinUI 颜色的 ARGB 记法

WinUI 主题资源里的颜色是 8 位十六进制 **AARRGGBB**（前两位是 alpha），不是网页常见的 RRGGBB：

| 原文 | 含义 | CSS 等效值 |
| --- | --- | --- |
| `#E4000000` | alpha=0xE4=228/255=0.894，纯黑 | `rgba(0, 0, 0, 0.894)` |
| `#B3FFFFFF` | alpha=0xB3=179/255=0.702，纯白 | `rgba(255, 255, 255, 0.702)` |
| `#4C3A3A3A` | alpha=0x4C=76/255=0.298，#3A3A3A | `rgba(58, 58, 58, 0.298)` |
| `#4D000000` | alpha=0x4D=77/255=0.302，纯黑 | `rgba(0, 0, 0, 0.302)` |

下文表格给出 ARGB 原文（官方）与换算后的 rgba（可直接抄进 CSS），换算保留 3 位小数。

### 0.3 未找到权威值的标注约定

格式：**未找到权威值，暂用 X（来源：Y）**。这类条目集中在：Acrylic 的模糊半径、Windows 系统 accent 的三态派生值、Tauri 窗口层的材质接线（Tauri 没有 system backdrop API）。

---

## 1. 材质（Material）

Fluent 2 官方把材质分为三类：**solid（实心）**、**occluding（遮挡型：Mica / Acrylic）**、**transparent（透明型：Smoke）**。
Mica / Acrylic 作为交互控件下方的**基础层（base layer）**出现；Smoke 用于强调凌驾其上的沉浸式表面。

### 1.1 五种材质总览

| 材质 | 类型 | 是否 mode aware | 色调来源 | 采样/计算成本 | 官方定位（原文摘句） |
| --- | --- | --- | --- | --- | --- |
| **Solid** | 不透明 | 是（light/dark） | 色板（neutral ramp） | 无 | the most common material … uses color and varying elevation |
| **Mica** | 不透明（opaque） | 是 | 桌面壁纸 + 主题，**只采样一次壁纸** | 低（专为性能设计） | opaque, dynamic material that incorporates theme and desktop wallpaper to paint the background of **long-lived windows** |
| **Mica Alt** | 不透明 | 是 | 同上，但**着色更强（stronger tinting）** | 低 | provides a deeper visual hierarchy than Mica, especially when creating an app with a **tabbed title bar** |
| **Acrylic** | 半透明（磨砂玻璃） | 是 | 背后内容（壁纸或应用内内容）+ tint | 高（GPU 密集） | semi-transparent material … Use it for **transient, light-dismiss surfaces** such as popovers and menus |
| **Smoke** | 透明遮罩 | **否**（永远半透明黑） | 固定黑 | 无 | dimming the surfaces beneath so that they recede into the background … signal **blocking interaction** below a modal UI |

### 1.2 用途边界（这是最容易做错的一节）

| 材质 | 可以用于 | **绝对不能用于** |
| --- | --- | --- |
| Mica | 应用窗口底（base layer）、标题栏区域 | 浮出层（flyout/menu/dialog）；同一次应用里叠加第二层 backdrop |
| Mica Alt | 带选项卡的标题栏、需要「标题栏 vs 命令区」有对比的窗口底 | 同上；不需要对比的普通窗口（用 Mica 即可） |
| Acrylic | 临时性、可轻触消失的表面：flyout、context menu、下拉、非模态弹层、NavigationView 的 Compact/Minimal 浮出面板 | **长驻内容的背景**（页面、列表、导航面板）；大面积背景；并排多块 acrylic（会产生可见接缝）；accent 色文字/超链接之上 |
| Smoke | 模态对话框（ContentDialog）背后的遮罩 | 任何非阻塞场景 |
| Layer / Card | 长驻内容层（Grid/StackPanel/Frame）、分区卡片 | 窗口底（那是 Mica 的位置） |

官方原文的 Do / Don't（来自 Mica 与 Acrylic 页，逐条翻译）：

- **Do**：把所有想露出 Mica 的层的背景设为 `transparent`。
- **Don't**：在一个应用里应用超过一次 backdrop material。
- **Don't**：把 backdrop material 应用到某个 UI 元素上（backdrop 只在它背后、直到窗口底之间的层都透明时才可见）。
- **Do**：Acrylic 至少延伸到应用的一条边，获得与四周融合的无缝感。
- **Don't**：把 desktop acrylic 放在应用的大面积背景上。
- **Don't**：并排放多个 acrylic 面板。
- **Don't**：在 acrylic 表面放 accent 色文字（14px 默认字号下几乎必然无法达到对比度要求），也尽量避免在 acrylic 上放超链接。

### 1.3 视觉配方

**Solid**：不透明纯色，来自 neutral ramp（见第 2 节）。

**Acrylic 的官方配方（5 层，来自 Windows Acrylic 页原文）**：

```text
1. background（背景内容）
2. blur（模糊）
3. exclusion blend mode layer（排除混合层 —— 保证其上方 UI 的对比度与可读性）
4. color / tint overlay（色调叠加）
5. noise（噪声纹理，供应商为材质增加物理质感）
```

原文：We started with translucency, blur, and noise to add visual depth and dimension to flat surfaces. We added an exclusion blend mode layer to ensure contrast and legibility of UI placed on an acrylic background. Finally, we added color tint for personalization opportunities.

**模糊半径 / 不透明度具体值：未找到权威值。** Fluent 2 官方 material 页与 Windows Acrylic 页均只描述配方结构，不给 px 与 alpha；WinUI 的 `DesktopAcrylicController` 暴露 `TintColor`、`TintOpacity`、`LuminosityOpacity`、`FallbackColor` 参数与 `DesktopAcrylicKind.Base / .Thin` 两种变体，但官方文档未列出默认数值。
实现侧建议（**非官方口径，来源：第三方 designmd.app 的 Fluent 2 token 表 + 常见 Windows 复刻实践**）：`blur(30px)` 对应 Base、`blur(60px)` 对应 Thin，噪声 alpha 约 0.02。落地时以系统渲染（Tauri 窗口用 `window-vibrancy` / Win32 `DwmSetWindowAttribute`）为准，不要用 CSS 硬凑。

**Smoke**：固定 `#4D000000` → `rgba(0, 0, 0, 0.302)`，light/dark 相同（官方明示 not mode aware）。

### 1.4 分层系统（Layering）

Windows 11 用**两层系统**：

- **base layer**：应用的地板。承载菜单、命令、导航相关的控件。
- **content layer**：把注意力引向应用的核心体验。可以是连续的一整块，也可以切成多张 card。

配合 Mica（不透明底）时，在 base 之上再加 content layer，用低不透明度纯色拾取下方材质：

| 层 | 官方填充资源 | 值（Light） | 值（Dark） | 应用对象 |
| --- | --- | --- | --- | --- |
| base（Mica） | `MicaBackdrop Kind=Base` | `SolidBackgroundFillColorBase` = `#F3F3F3` | `#202020` | 窗口底 |
| base（Mica Alt） | `MicaBackdrop Kind=BaseAlt` | `SolidBackgroundFillColorBaseAlt` = `#DADADA` | `#0A0A0A` | 带选项卡标题栏的窗口底 |
| content layer | `LayerFillColorDefaultBrush` | `#80FFFFFF` → `rgba(255,255,255,0.502)` | `#4C3A3A3A` → `rgba(58,58,58,0.298)` | Grid / StackPanel / Frame 等容器背景 |
| commanding layer（仅 Mica Alt 体系） | `LayerOnMicaBaseAltFillColorDefaultBrush` | `#B3FFFFFF` → `rgba(255,255,255,0.702)` | `#733A3A3A` → `rgba(58,58,58,0.451)` | MenuBar、导航结构等命令区 |
| card | `CardBackgroundFillColorDefaultBrush` + `CardStrokeColorDefaultBrush` | `#B3FFFFFF` + 边框 `#0F000000` | `#0DFFFFFF` + 边框 `#19000000` | 分区卡片 |

两种官方内容层模式：

- **Standard pattern**：一整块连续背景，用于需要和 base 层形成层级差异的大面积区域。
- **Card pattern**：多张分段的卡片，用于由多个不连续区块构成的应用。用 Card pattern 时，需要覆盖 NavigationView 默认的 content layer 背景与边框主题资源，再在内容区自己造卡片。

**标题栏**：想让窗口看起来无缝，Mica 必须露出到标题栏 —— 做法是把应用延伸到 non-client 区域并使用 `transparent` 的自定义标题栏。

### 1.5 降级与禁用条件（必须实现）

| 触发条件 | Mica / Mica Alt 的行为 | Acrylic 的行为 |
| --- | --- | --- |
| 系统设置里关掉「透明效果」 | 退回 `SolidBackgroundFillColorBase`（Alt 为 `SolidBackgroundFillColorBaseAlt`） | 退回纯色 |
| 节电模式（Battery Saver） | 同上退回纯色 | 同上退回纯色 |
| 低端硬件 | 同上退回纯色 | 同上退回纯色 |
| 窗口处于非活动状态（桌面应用） | 退回中性色（Mica 内置表示窗口焦点） | **仅 background acrylic** 退回纯色 |
| 高对比度主题 | 使用用户选择的高对比度背景色 | 同左 |
| Windows 版本低于 22000 | 退回纯色 | 不适用 |
| Xbox / HoloLens / 平板模式 | 不适用 | **仅 background acrylic** 退回纯色 |


---

## 2. 颜色

### 2.1 三层体系

| 层 | 作用 | 例子 |
| --- | --- | --- |
| **global token** | 与语境无关的**原始值**（hex、px、ms） | `grey[96] = #f5f5f5`、`brandWeb[80] = #0f6cbd` |
| **alias token** | 给原始值加上**语义**（用在哪、什么状态） | `colorNeutralBackground1`、`colorBrandBackgroundHover` |
| **component token** | 具体控件的槽位 | `ButtonBackground` → `ControlFillColorDefaultBrush` |

Fluent 2 定义三套调色板：**neutral**（黑/白/灰，用于 surface、text、布局元素，常以颜色变化表达状态）、**shared**（跨 M365 对齐，用于 avatar/日历/badge；深色模式下会调整饱和度与亮度）、**brand**（产品/品牌色，避免大面积或过度使用）。

### 2.2 Neutral ramp（Fluent 2 global，官方 `grey`，共 51 档）

官方注释明示：这些数值 **适用于 light 与 dark 两种模式** —— 浅色主题取高端（96/98/white），深色主题取低端（8/12/16）。

| key | hex | key | hex | key | hex |
| --- | --- | --- | --- | --- | --- |
| `grey[2]` | #050505 | `grey[4]` | #0a0a0a | `grey[6]` | #0f0f0f |
| `grey[8]` | #141414 | `grey[10]` | #1a1a1a | `grey[12]` | #1f1f1f |
| `grey[14]` | #242424 | `grey[16]` | #292929 | `grey[18]` | #2e2e2e |
| `grey[20]` | #333333 | `grey[22]` | #383838 | `grey[24]` | #3d3d3d |
| `grey[26]` | #424242 | `grey[28]` | #474747 | `grey[30]` | #4d4d4d |
| `grey[32]` | #525252 | `grey[34]` | #575757 | `grey[36]` | #5c5c5c |
| `grey[38]` | #616161 | `grey[40]` | #666666 | `grey[42]` | #6b6b6b |
| `grey[44]` | #707070 | `grey[46]` | #757575 | `grey[48]` | #7a7a7a |
| `grey[50]` | #808080 | `grey[52]` | #858585 | `grey[54]` | #8a8a8a |
| `grey[56]` | #8f8f8f | `grey[58]` | #949494 | `grey[60]` | #999999 |
| `grey[62]` | #9e9e9e | `grey[64]` | #a3a3a3 | `grey[66]` | #a8a8a8 |
| `grey[68]` | #adadad | `grey[70]` | #b3b3b3 | `grey[72]` | #b8b8b8 |
| `grey[74]` | #bdbdbd | `grey[76]` | #c2c2c2 | `grey[78]` | #c7c7c7 |
| `grey[80]` | #cccccc | `grey[82]` | #d1d1d1 | `grey[84]` | #d6d6d6 |
| `grey[86]` | #dbdbdb | `grey[88]` | #e0e0e0 | `grey[90]` | #e6e6e6 |
| `grey[92]` | #ebebeb | `grey[94]` | #f0f0f0 | `grey[96]` | #f5f5f5 |
| `grey[98]` | #fafafa | `grey[99]` | #fcfcfc |

另有：`white = #ffffff`、`black = #000000`。

alpha 系列（各 10 档：5/10/20/30/40/50/60/70/80/90，均为百分比 alpha）：

| 系列 | 取自 | 示例 |
| --- | --- | --- |
| `whiteAlpha` | 白 + alpha | `whiteAlpha[50] = rgba(255, 255, 255, 0.5)` |
| `blackAlpha` | 黑 + alpha | `blackAlpha[40] = rgba(0, 0, 0, 0.4)` |
| `grey10Alpha` | #1a1a1a + alpha | `grey10Alpha[50] = rgba(26, 26, 26, 0.5)` |
| `grey12Alpha` | #1f1f1f + alpha | `grey12Alpha[70] = rgba(31, 31, 31, 0.7)` |
| `grey14Alpha` | #242424 + alpha | `grey14Alpha[80] = rgba(36, 36, 36, 0.8)` |

### 2.3 Brand ramp（官方 `brandWeb`，16 档 10–160）与 accent 交互态

| token | hex |
| --- | --- |
| `brandWeb[10]` | #061724 |
| `brandWeb[20]` | #082338 |
| `brandWeb[30]` | #0a2e4a |
| `brandWeb[40]` | #0c3b5e |
| `brandWeb[50]` | #0e4775 |
| `brandWeb[60]` | #0f548c |
| `brandWeb[70]` | #115ea3 |
| `brandWeb[80]` | #0f6cbd |
| `brandWeb[90]` | #2886de |
| `brandWeb[100]` | #479ef5 |
| `brandWeb[110]` | #62abf5 |
| `brandWeb[120]` | #77b7f7 |
| `brandWeb[130]` | #96c6fa |
| `brandWeb[140]` | #b4d6fa |
| `brandWeb[150]` | #cfe4fa |
| `brandWeb[160]` | #ebf3fc |

其它官方 brand ramp：`brandTeams`（80 = `#5b5fc7`）、`brandOffice`（80 = `#d83b01`）、`brandTeamsV21`（80 = `#654cf5`）。AudioLink 用默认 `brandWeb`。

**accent / brand 的默认与交互态（alias token → brand ramp 档位 → brandWeb 值）**：

| Fluent 2 alias token | 映射 | Light 主题值 | Dark 主题值 |
| --- | --- | --- | --- |
| `colorBrandBackground` | light: `brand[80]` / dark: `brand[70]` | `#0f6cbd` | `#115ea3` |
| `colorBrandBackgroundHover` | light: `brand[70]` / dark: `brand[80]` | `#115ea3` | `#0f6cbd` |
| `colorBrandBackgroundPressed` | `brand[40]` | `#0c3b5e` | `#0c3b5e` |
| `colorBrandBackgroundSelected` | `brand[60]` | `#0f548c` | `#0f548c` |
| `colorCompoundBrandBackground`（复选框/开关/滑块等复合适配） | light: `brand[80]` / dark: `brand[100]` | `#0f6cbd` | `#479ef5` |
| `colorCompoundBrandBackgroundHover` | light: `brand[70]` / dark: `brand[110]` | `#115ea3` | `#62abf5` |
| `colorCompoundBrandBackgroundPressed` | light: `brand[60]` / dark: `brand[90]` | `#0f548c` | `#2886de` |
| `colorBrandForeground1`（链接/图标） | light: `brand[80]` / dark: `brand[100]` | `#0f6cbd` | `#479ef5` |
| `colorBrandForegroundLink` | light: `brand[70]` / dark: `brand[100]` | `#115ea3` | `#479ef5` |
| `colorNeutralForegroundOnBrand`（accent 底上的文字） | `white` | `#ffffff` | `#ffffff` |

> 注意坑：官方 `lightColor.ts` / `darkColor.ts` 里每条后面的行尾注释（例如 `// #106ebe Global.Color.Brand.70`）是用 **Office/Windows brand ramp** 生成的，与默认 `brandWeb` 的取值不同。**以本文的映射表为准，不要抄注释里的 hex。**

**Windows 11 系统 accent（WinUI 3 口径）**：

| WinUI token | 取值规则 | 备注 |
| --- | --- | --- |
| `SystemAccentColor` | 用户在系统设置里选的 accent；**Windows 出厂默认 `#0078D4`** | 运行时会变 |
| `AccentFillColorDefault` | = `SystemAccentColor` | accent 按钮/开关底色 |
| `AccentFillColorSecondary` | = accent 90% 不透明（hover） | **未找到公开 XAML 定义**；官方主题资源里只导出 `AccentFillColorDisabled = #37000000`(light) / `#28FFFFFF`(dark)。三态派生规则为 Windows 一贯口径，实现时以系统 API / WinUI 运行时为准 |
| `AccentFillColorTertiary` | = accent 80% 不透明（pressed） | 同上 |

**Fluent 2 与 Windows 11 的 accent 默认值不同**：Fluent 2（web）默认 `#0f6cbd`，Windows 11 系统 accent 默认 `#0078d4`。AudioLink 若走 Tauri 原生窗口，建议**跟随系统 accent**（Win32 `DwmGetColorizationColor` / 主题 API），把 `#0f6cbd` 作为回退。

### 2.4 Fluent 2 alias 颜色全表（Light / Dark）

下表为官方 `lightColor.ts` / `darkColor.ts` 中**与界面结构直接相关**的 token。语义色见 2.6。

| alias token | Light | Dark | 用途 |
| --- | --- | --- | --- |
| `colorNeutralForeground1` | `#242424` | `#ffffff` | 主要文字 |
| `colorNeutralForeground2` | `#424242` | `#d6d6d6` | 次级文字 |
| `colorNeutralForeground3` | `#616161` | `#adadad` | 三级文字/图标 |
| `colorNeutralForeground4` | `#707070` | `#999999` | 最弱文字 |
| `colorNeutralForegroundDisabled` | `#bdbdbd` | `#5c5c5c` | 禁用文字 |
| `colorNeutralForegroundInverted` | `#ffffff` | `#242424` | 反色文字 |
| `colorNeutralBackground1` | `#ffffff` | `#292929` | 主表面（card、输入框） |
| `colorNeutralBackground1Hover` | `#f5f5f5` | `#3d3d3d` | 主表面 hover |
| `colorNeutralBackground1Pressed` | `#e0e0e0` | `#1f1f1f` | 主表面 pressed |
| `colorNeutralBackground1Selected` | `#ebebeb` | `#383838` | 主表面 selected |
| `colorNeutralBackground2` | `#fafafa` | `#1f1f1f` | 次级表面 |
| `colorNeutralBackground3` | `#f5f5f5` | `#141414` | 三级表面 |
| `colorNeutralBackground3Hover` | `#ebebeb` | `#292929` | 三级表面 hover |
| `colorNeutralBackground4` | `#f0f0f0` | `#0a0a0a` | 四级表面 |
| `colorNeutralBackground6` | `#e6e6e6` | `#333333` | 静态大面积表面 |
| `colorNeutralBackgroundDisabled` | `#f0f0f0` | `#141414` | 禁用表面 |
| `colorNeutralCardBackground` | `#fafafa` | `#333333` | 卡片底 |
| `colorNeutralCardBackgroundHover` | `#ffffff` | `#3d3d3d` | 卡片 hover |
| `colorNeutralStencil1` | `#e6e6e6` | `#575757` | 占位块 A |
| `colorNeutralStencil2` | `#fafafa` | `#333333` | 占位块 B |
| `colorSubtleBackground` | `transparent` | `transparent` | subtle 控件默认（无底） |
| `colorSubtleBackgroundHover` | `#f5f5f5` | `#383838` | subtle hover |
| `colorSubtleBackgroundPressed` | `#e0e0e0` | `#2e2e2e` | subtle pressed |
| `colorSubtleBackgroundSelected` | `#ebebeb` | `#333333` | subtle selected |
| `colorNeutralStroke1` | `#d1d1d1` | `#666666` | 一级描边（控件边界） |
| `colorNeutralStroke1Hover` | `#c7c7c7` | `#757575` | 一级描边 hover |
| `colorNeutralStroke1Pressed` | `#b3b3b3` | `#6b6b6b` | 一级描边 pressed |
| `colorNeutralStroke2` | `#e0e0e0` | `#525252` | 二级描边 |
| `colorNeutralStroke3` | `#f0f0f0` | `#3d3d3d` | 三级描边（分隔线） |
| `colorNeutralStrokeAccessible` | `#616161` | `#adadad` | 需要满足对比度的描边（输入框） |
| `colorNeutralStrokeDisabled` | `#e0e0e0` | `#424242` | 禁用描边 |
| `colorBrandStroke1` | `#0f6cbd` | `#479ef5` | 品牌描边 |
| `colorCompoundBrandStroke` | `#0f6cbd` | `#479ef5` | 复合控件选中描边 |
| `colorNeutralStrokeOnBrand` | `#ffffff` | `#292929` | accent 底上的描边 |
| `colorBackgroundOverlay` | `rgba(0,0,0,0.4)` | `rgba(0,0,0,0.5)` | 模态遮罩（对应 WinUI Smoke） |
| `colorScrollbarOverlay` | `rgba(0,0,0,0.5)` | `rgba(255,255,255,0.6)` | 滚动条 |
| `colorStrokeFocus1` | `#ffffff` | `#000000` | 焦点环外圈 |
| `colorStrokeFocus2` | `#000000` | `#ffffff` | 焦点环内圈 |
| `colorNeutralShadowAmbient` | `rgba(0,0,0,0.12)` | `rgba(0,0,0,0.24)` | 环境阴影（ambient） |
| `colorNeutralShadowKey` | `rgba(0,0,0,0.14)` | `rgba(0,0,0,0.28)` | 投射阴影（key） |

### 2.5 WinUI 3 alias 颜色全表（Windows 11 原生口径，83 条）

这是 `Common_themeresources_any.xaml` 中实际导出的全部颜色资源（Light / Dark 两套，HighContrast 另有 83 条）。**这是和系统窗口、Mica 层对齐时应该用的 token。**

| WinUI token | Light（ARGB / rgba） | Dark（ARGB / rgba） |
| --- | --- | --- |
| `TextFillColorPrimary` | #E4000000 / rgba(0,0,0,0.894) | #FFFFFFFF / #ffffff |
| `TextFillColorSecondary` | #9E000000 / rgba(0,0,0,0.62) | #C5FFFFFF / rgba(255,255,255,0.773) |
| `TextFillColorTertiary` | #72000000 / rgba(0,0,0,0.447) | #87FFFFFF / rgba(255,255,255,0.529) |
| `TextFillColorDisabled` | #5C000000 / rgba(0,0,0,0.361) | #5DFFFFFF / rgba(255,255,255,0.365) |
| `TextFillColorInverse` | #FFFFFF / #ffffff | #E4000000 / rgba(0,0,0,0.894) |
| `AccentTextFillColorDisabled` | #5C000000 / rgba(0,0,0,0.361) | #5DFFFFFF / rgba(255,255,255,0.365) |
| `TextOnAccentFillColorPrimary` | #FFFFFF / #ffffff | #000000 / #000000 |
| `TextOnAccentFillColorSecondary` | #B3FFFFFF / rgba(255,255,255,0.702) | #80000000 / rgba(0,0,0,0.502) |
| `TextOnAccentFillColorDisabled` | #FFFFFF / #ffffff | #87FFFFFF / rgba(255,255,255,0.529) |
| `TextOnAccentFillColorSelectedText` | #FFFFFF / #ffffff | #FFFFFF / #ffffff |
| `ControlFillColorDefault` | #B3FFFFFF / rgba(255,255,255,0.702) | #0FFFFFFF / rgba(255,255,255,0.059) |
| `ControlFillColorSecondary` | #80F9F9F9 / rgba(249,249,249,0.502) | #15FFFFFF / rgba(255,255,255,0.082) |
| `ControlFillColorTertiary` | #4DF9F9F9 / rgba(249,249,249,0.302) | #08FFFFFF / rgba(255,255,255,0.031) |
| `ControlFillColorQuarternary` | #C2F3F3F3 / rgba(243,243,243,0.761) | #0FFFFFFF / rgba(255,255,255,0.059) |
| `ControlFillColorDisabled` | #4DF9F9F9 / rgba(249,249,249,0.302) | #0BFFFFFF / rgba(255,255,255,0.043) |
| `ControlFillColorTransparent` | #00FFFFFF / transparent | #00FFFFFF / transparent |
| `ControlFillColorInputActive` | #FFFFFF / #ffffff | #B31E1E1E / rgba(30,30,30,0.702) |
| `ControlSolidFillColorDefault` | #FFFFFF / #ffffff | #454545 / #454545 |
| `ControlStrongFillColorDefault` | #72000000 / rgba(0,0,0,0.447) | #8BFFFFFF / rgba(255,255,255,0.545) |
| `ControlStrongFillColorDisabled` | #51000000 / rgba(0,0,0,0.318) | #3FFFFFFF / rgba(255,255,255,0.247) |
| `SubtleFillColorTransparent` | #00FFFFFF / transparent | #00FFFFFF / transparent |
| `SubtleFillColorSecondary` | #09000000 / rgba(0,0,0,0.035) | #0FFFFFFF / rgba(255,255,255,0.059) |
| `SubtleFillColorTertiary` | #06000000 / rgba(0,0,0,0.024) | #0AFFFFFF / rgba(255,255,255,0.039) |
| `SubtleFillColorDisabled` | #00FFFFFF / transparent | #00FFFFFF / transparent |
| `ControlAltFillColorTransparent` | #00FFFFFF / transparent | #00FFFFFF / transparent |
| `ControlAltFillColorSecondary` | #06000000 / rgba(0,0,0,0.024) | #19000000 / rgba(0,0,0,0.098) |
| `ControlAltFillColorTertiary` | #0F000000 / rgba(0,0,0,0.059) | #0BFFFFFF / rgba(255,255,255,0.043) |
| `ControlAltFillColorQuarternary` | #18000000 / rgba(0,0,0,0.094) | #12FFFFFF / rgba(255,255,255,0.071) |
| `ControlAltFillColorDisabled` | #00FFFFFF / transparent | #00FFFFFF / transparent |
| `ControlOnImageFillColorDefault` | #C9FFFFFF / rgba(255,255,255,0.788) | #B31C1C1C / rgba(28,28,28,0.702) |
| `ControlOnImageFillColorSecondary` | #F3F3F3 / #f3f3f3 | #1A1A1A / #1a1a1a |
| `ControlOnImageFillColorTertiary` | #EBEBEB / #ebebeb | #131313 / #131313 |
| `ControlOnImageFillColorDisabled` | #00FFFFFF / transparent | #1E1E1E / #1e1e1e |
| `AccentFillColorDisabled` | #37000000 / rgba(0,0,0,0.216) | #28FFFFFF / rgba(255,255,255,0.157) |
| `ControlStrokeColorDefault` | #0F000000 / rgba(0,0,0,0.059) | #12FFFFFF / rgba(255,255,255,0.071) |
| `ControlStrokeColorSecondary` | #29000000 / rgba(0,0,0,0.161) | #18FFFFFF / rgba(255,255,255,0.094) |
| `ControlStrokeColorOnAccentDefault` | #14FFFFFF / rgba(255,255,255,0.078) | #14FFFFFF / rgba(255,255,255,0.078) |
| `ControlStrokeColorOnAccentSecondary` | #66000000 / rgba(0,0,0,0.4) | #23000000 / rgba(0,0,0,0.137) |
| `ControlStrokeColorOnAccentTertiary` | #37000000 / rgba(0,0,0,0.216) | #37000000 / rgba(0,0,0,0.216) |
| `ControlStrokeColorOnAccentDisabled` | #0F000000 / rgba(0,0,0,0.059) | #33000000 / rgba(0,0,0,0.2) |
| `ControlStrongStrokeColorDefault` | #72000000 / rgba(0,0,0,0.447) | #8BFFFFFF / rgba(255,255,255,0.545) |
| `ControlStrongStrokeColorDisabled` | #37000000 / rgba(0,0,0,0.216) | #28FFFFFF / rgba(255,255,255,0.157) |
| `ControlStrokeColorForStrongFillWhenOnImage` | #59FFFFFF / rgba(255,255,255,0.349) | #6B000000 / rgba(0,0,0,0.42) |
| `CardBackgroundFillColorDefault` | #B3FFFFFF / rgba(255,255,255,0.702) | #0DFFFFFF / rgba(255,255,255,0.051) |
| `CardBackgroundFillColorSecondary` | #80F6F6F6 / rgba(246,246,246,0.502) | #08FFFFFF / rgba(255,255,255,0.031) |
| `CardBackgroundFillColorTertiary` | #FFFFFF / #ffffff | #12FFFFFF / rgba(255,255,255,0.071) |
| `CardStrokeColorDefault` | #0F000000 / rgba(0,0,0,0.059) | #19000000 / rgba(0,0,0,0.098) |
| `CardStrokeColorDefaultSolid` | #EBEBEB / #ebebeb | #1C1C1C / #1c1c1c |
| `SurfaceStrokeColorDefault` | #66757575 / rgba(117,117,117,0.4) | #66757575 / rgba(117,117,117,0.4) |
| `SurfaceStrokeColorFlyout` | #0F000000 / rgba(0,0,0,0.059) | #33000000 / rgba(0,0,0,0.2) |
| `SurfaceStrokeColorInverse` | #15FFFFFF / rgba(255,255,255,0.082) | #0F000000 / rgba(0,0,0,0.059) |
| `DividerStrokeColorDefault` | #0F000000 / rgba(0,0,0,0.059) | #15FFFFFF / rgba(255,255,255,0.082) |
| `FocusStrokeColorOuter` | #E4000000 / rgba(0,0,0,0.894) | #FFFFFF / #ffffff |
| `FocusStrokeColorInner` | #B3FFFFFF / rgba(255,255,255,0.702) | #B3000000 / rgba(0,0,0,0.702) |
| `SmokeFillColorDefault` | #4D000000 / rgba(0,0,0,0.302) | #4D000000 / rgba(0,0,0,0.302) |
| `LayerFillColorDefault` | #80FFFFFF / rgba(255,255,255,0.502) | #4C3A3A3A / rgba(58,58,58,0.298) |
| `LayerFillColorAlt` | #FFFFFF / #ffffff | #0DFFFFFF / rgba(255,255,255,0.051) |
| `LayerOnAcrylicFillColorDefault` | #40FFFFFF / rgba(255,255,255,0.251) | #09FFFFFF / rgba(255,255,255,0.035) |
| `LayerOnAccentAcrylicFillColorDefault` | #40FFFFFF / rgba(255,255,255,0.251) | #09FFFFFF / rgba(255,255,255,0.035) |
| `LayerOnMicaBaseAltFillColorDefault` | #B3FFFFFF / rgba(255,255,255,0.702) | #733A3A3A / rgba(58,58,58,0.451) |
| `LayerOnMicaBaseAltFillColorSecondary` | #0A000000 / rgba(0,0,0,0.039) | #0FFFFFFF / rgba(255,255,255,0.059) |
| `LayerOnMicaBaseAltFillColorTertiary` | #F9F9F9 / #f9f9f9 | #2C2C2C / #2c2c2c |
| `LayerOnMicaBaseAltFillColorTransparent` | #00000000 / transparent | #00FFFFFF / transparent |
| `SolidBackgroundFillColorBase` | #F3F3F3 / #f3f3f3 | #202020 / #202020 |
| `SolidBackgroundFillColorBaseAlt` | #DADADA / #dadada | #0A0A0A / #0a0a0a |
| `SolidBackgroundFillColorSecondary` | #EEEEEE / #eeeeee | #1C1C1C / #1c1c1c |
| `SolidBackgroundFillColorTertiary` | #F9F9F9 / #f9f9f9 | #282828 / #282828 |
| `SolidBackgroundFillColorQuarternary` | #FFFFFF / #ffffff | #2C2C2C / #2c2c2c |
| `SolidBackgroundFillColorQuinary` | #FDFDFD / #fdfdfd | #333333 / #333333 |
| `SolidBackgroundFillColorSenary` | #FFFFFF / #ffffff | #373737 / #373737 |
| `SolidBackgroundFillColorTransparent` | #00F3F3F3 / transparent | #00202020 / transparent |
| `SystemFillColorNeutral` | #72000000 / rgba(0,0,0,0.447) | #8BFFFFFF / rgba(255,255,255,0.545) |
| `SystemFillColorSolidNeutral` | #8A8A8A / #8a8a8a | #9D9D9D / #9d9d9d |
| `SystemFillColorAttentionBackground` | #80F6F6F6 / rgba(246,246,246,0.502) | #08FFFFFF / rgba(255,255,255,0.031) |
| `SystemFillColorSolidAttentionBackground` | #F7F7F7 / #f7f7f7 | #2E2E2E / #2e2e2e |
| `SystemFillColorSolidNeutralBackground` | #F3F3F3 / #f3f3f3 | #2E2E2E / #2e2e2e |

（`SystemFillColorSuccess / Caution / Critical` 与对应背景色见下一节。）

### 2.6 语义色：success / caution / critical

**A. Windows 11 / WinUI 3（前台色 + 背景色成对）**

| WinUI token | Light | Dark |
| --- | --- | --- |
| `SystemFillColorSuccess` | `#0F7B0F` | `#6CCB5F` |
| `SystemFillColorCaution` | `#9D5D00` | `#FCE100` |
| `SystemFillColorCritical` | `#C42B1C` | `#FF99A4` |
| `SystemFillColorSuccessBackground` | `#DFF6DD` | `#393D1B` |
| `SystemFillColorCautionBackground` | `#FFF4CE` | `#433519` |
| `SystemFillColorCriticalBackground` | `#FDE7E9` | `#442726` |
| `SystemFillColorNeutralBackground` | `#06000000` / rgba(0,0,0,0.024) | `#08FFFFFF` / rgba(255,255,255,0.031) |

**B. Fluent 2（Fluent UI v9）语义/状态 token**

规则（源码 `lightColorPalette.ts` / `darkColorPalette.ts`）：`Background1 = tint60`（浅色主题）/ `shade40`（深色主题）；`Background2 = tint40` / `shade30`；`Background3 = primary`；`Foreground1 = shade10` / `tint30`；`Foreground2 = shade30` / `tint40`；`Border1 = tint40` / `primary`；`Border2 = primary` / `tint20`。映射为 `success → green`、`warning → orange`、`danger → cranberry`（官方 `mappedStatusColors`）。

| token | Light | Dark |
| --- | --- | --- |
| `colorStatusSuccessBackground1` | `#f1faf1` | `#052505` |
| `colorStatusSuccessBackground2` | `#9fd89f` | `#094509` |
| `colorStatusSuccessBackground3` | `#107c10` | `#107c10` |
| `colorStatusSuccessForeground1` | `#0e700e` | `#54b054` |
| `colorStatusSuccessForeground2` | `#094509` | `#9fd89f` |
| `colorStatusSuccessForeground3` | `#107c10` | `#9fd89f` |
| `colorStatusSuccessBorder1` | `#9fd89f` | `#107c10` |
| `colorStatusSuccessBorder2` | `#107c10` | `#9fd89f` |
| `colorStatusWarningBackground1` | `#fff9f5` | `#4a1e04` |
| `colorStatusWarningBackground2` | `#fdcfb4` | `#8a3707` |
| `colorStatusWarningBackground3` | `#f7630c` | `#f7630c` |
| `colorStatusWarningForeground1` | `#bc4b09` | `#faa06b` |
| `colorStatusWarningForeground2` | `#8a3707` | `#fdcfb4` |
| `colorStatusWarningForeground3` | `#bc4b09` | `#f98845` |
| `colorStatusWarningBorder1` | `#fdcfb4` | `#f7630c` |
| `colorStatusWarningBorder2` | `#bc4b09` | `#f98845` |
| `colorStatusDangerBackground1` | `#fdf3f4` | `#3b0509` |
| `colorStatusDangerBackground2` | `#eeacb2` | `#6e0811` |
| `colorStatusDangerBackground3` | `#c50f1f` | `#c50f1f` |
| `colorStatusDangerForeground1` | `#b10e1c` | `#dc626d` |
| `colorStatusDangerForeground2` | `#6e0811` | `#eeacb2` |
| `colorStatusDangerForeground3` | `#c50f1f` | `#eeacb2` |
| `colorStatusDangerBorder1` | `#eeacb2` | `#c50f1f` |
| `colorStatusDangerBorder2` | `#c50f1f` | `#dc626d` |

> 上表数值由官方源码的映射规则 + 官方 shared color 变体（`green` / `orange` / `cranberry` 的 `tint60..shade30`）推导得出，规则与变量均来自官方仓库；推导过程可复核：`mappedStatusColors = { cranberry, green, orange }`。

### 2.7 交互状态的通用规则

- Fluent 2：**组件越交互越深** —— rest（最浅）→ hover（更深）→ selected（最深）。
- **Platform distinction（官方明示）**：Windows 的交互状态当前是**反向的** —— 控件在交互时**变亮**。做 Windows 桌面应用时按 Windows 的口径（例如 `ControlFillColorSecondary` 比 `ControlFillColorDefault` 在 light 下更亮 0.502 vs 0.702 白 → 实际看起来更「浅/透」，dark 下更亮）。
- **focus 态不改变控件颜色**，而是容器获得**更粗的描边**，以便明确区分鼠标与键盘交互。
- 无障碍：常规文字对背景对比度 ≥ 4.5:1；大字（bold > 18.5px 或 regular > 24px）≥ 3:1；不要只靠颜色传达信息。


---

## 3. 排版（Typography）

### 3.1 字体：Segoe UI Variable 与 optical size

Windows 的系统字体是 **Segoe UI Variable**，两个可变轴：

- **weight（`wght`）**：Thin(100) → Bold(700) 连续可变。
- **optical size（`opsz`）**：**自动开启**，为 8pt–36pt 的光学校准区间控制字怀（counter）形状与大小 —— 小尺寸优先可读性，大尺寸优先个性。

| 变体 | 何时用 | 触发方式 |
| --- | --- | --- |
| `Segoe UI Variable Small` | 小字号（ui 注释为 small，对应 Caption 12px 档） | 可变字体自动按 font-size 匹配 opsz（XAML 与 HTML 均自动） |
| `Segoe UI Variable Text` | 正文（14/18px） | 同上 |
| `Segoe UI Variable Display` | 大标题（20px 以上：Subtitle / Title / Title Large / Display） | 同上 |

官方原文：Segoe UI Variable uses variable font technology to dynamically provide great legibility at very small sizes, and improved outlines at display sizes；When using HTML, optical scaling is also automatic, but you will need to specify the Segoe UI Variable font in CSS。

CSS 侧字体栈（AudioLink 建议）：

```css
--font-ui: 'Segoe UI Variable Text', 'Segoe UI Variable', 'Segoe UI', system-ui, sans-serif;
--font-display: 'Segoe UI Variable Display', 'Segoe UI Variable', 'Segoe UI', sans-serif;
--font-mono: 'Cascadia Code', Consolas, 'Courier New', monospace;
```

字重可用档位（官方 weight 表）：Light 300 / Semilight 350 / Regular 400 / **Semibold 600** / Bold 700。
Fluent 2 的 alias 字重 token：`fontWeightRegular: 400`、`fontWeightMedium: 500`、`fontWeightSemibold: 600`、`fontWeightBold: 700`。

### 3.2 Windows type ramp（Segoe UI Variable，官方全表）

| 档位名 | 字重（weight） | 字号/行高（epx） | 官方 XAML style | 用途 |
| --- | --- | --- | --- | --- |
| **Caption** | Regular small | 12 / 16 | `CaptionTextBlockStyle` | 最小辅助信息、时间戳、注释（低于 12px Regular 在部分语言不可读 → 官方最小尺寸限制） |
| **Body** | Regular (Text) | 14 / 20 | `BodyTextBlockStyle` | **默认正文**、控件标签、列表项 |
| **Body Strong** | Semibold (Text semibold) | 14 / 20 | `BodyStrongTextBlockStyle` | 正文中的强调（**用 Semibold，不用 Bold**） |
| **Body Large** | Regular (Text) | 18 / 24 | `BodyLargeTextBlockStyle` | 引导文字、大段说明 |
| **Body Large Strong** | Semibold | 18 / 24 | `BodyLargeStrongTextBlockStyle` | 大段说明中的强调 |
| **Subtitle** | Semibold (Display semibold) | 20 / 28 | `SubtitleTextBlockStyle` | 分组标题、对话框主标题 |
| **Title** | Semibold (Display semibold) | 28 / 36 | `TitleTextBlockStyle` | 页面标题 |
| **Title Large** | Semibold | 40 / 52 | `TitleLargeTextBlockStyle` | 首屏/空状态主标题 |
| **Display** | Semibold | 68 / 92 | `DisplayTextBlockStyle` | 英雄区大字（桌面应用极少用） |

官方规则（来自 Windows 排版页）：

- 字重：**Regular 用于绝大多数文字，Semibold 用于标题**。
- **最小尺寸**：14px Semibold / 12px Regular —— 再小在部分语言不可读。
- **大小写**：一律 **sentence case**（包括标题）。
- **对齐**：默认左对齐；仅少数场景（如图标下方文字）居中。
- **截断**：多数场景用省略号；官方对正文容器另有一说 —— 容器边界不明确或带「查看更多」链接时用省略号，否则可裁剪并换行。
- **Bold 与 Italic 不属于 Windows type ramp** —— 强调用 Semibold；不用 Italic（会降低可读性，尤其对阅读障碍用户）。
- 每行 **50–60 字符**为宜；少于 20 或多于 60 都难读。

### 3.3 Fluent 2 Web type ramp（跨平台口径）

| 档位名 | 字重 | 字号/行高 | 对应 global token |
| --- | --- | --- | --- |
| **Caption 2** | Regular | 10 / 14 | `fontSizeBase100` / `lineHeightBase100` |
| **Caption 2 Strong** | Semibold | 10 / 14 | 同上 |
| **Caption 1** | Regular | 12 / 16 | `fontSizeBase200` / `lineHeightBase200` |
| **Caption 1 Strong** | Semibold | 12 / 16 | 同上 |
| **Caption 1 Stronger** | Bold | 12 / 16 | 同上 |
| **Body 1** | Regular | 14 / 20 | `fontSizeBase300` / `lineHeightBase300` |
| **Body 1 Strong** | Semibold | 14 / 20 | 同上 |
| **Body 1 Stronger** | Bold | 14 / 20 | 同上 |
| **Subtitle 2** | Semibold | 16 / 22 | `fontSizeBase400` / `lineHeightBase400` |
| **Subtitle 2 Stronger** | Bold | 16 / 22 | 同上 |
| **Subtitle 1** | Semibold | 20 / 26 | `fontSizeBase500` / `lineHeightBase500` |
| **Title 3** | Semibold | 24 / 32 | `fontSizeBase600` / `lineHeightBase600` |
| **Title 2** | Semibold | 28 / 36 | `fontSizeHero700` / `lineHeightHero700` |
| **Title 1** | Semibold | 32 / 40 | `fontSizeHero800` / `lineHeightHero800` |
| **Large Title** | Semibold | 40 / 52 | `fontSizeHero900` / `lineHeightHero900` |
| **Display** | Semibold | 68 / 92 | `fontSizeHero1000` / `lineHeightHero1000` |

组装规则：`typographyStyles` 里的每个样式 = fontFamilyBase + fontSize + fontWeight + lineHeight 四件套（官方 `global/typographyStyles.ts`）。

**Windows 与 Web 的差异点**：Windows 侧的 Subtitle 是 20/28、Web 侧 Subtitle 1 是 20/26；Windows 没有 Caption 2（10px 档）；Windows 用 semibold 的显示字重档（Display semibold）描述 Subtitle 及以上。做 Windows 桌面应用时**以 3.2 的 Windows ramp 为准**。

### 3.4 排版禁忌（官方）

- 不用 all caps 吸引注意（难读）。
- 不用 Bold 作强调（Windows ramp 用 Semibold）；不用 Italic。
- 不对长段 LTR 文字右对齐；居中只用于强调短文案或配合其他元素。
- 不使用 50–60 字符以外的行宽作为正文主排版宽度。


---

## 4. 形状与层级（Shape & Elevation）

### 4.1 圆角

**Windows 11 / WinUI 3（权威表，来自 Geometry 页）**

| 圆角值 | 用途 | 覆盖控件 |
| --- | --- | --- |
| **8px** | 顶层容器：应用窗口、flyout、dialog、ContentDialog、MenuFlyout、TeachingTip | `OverlayCornerRadius`（默认 `8,8,8,8`） |
| **4px** | 页面内元素：Button、CheckBox、ComboBox、TextBox、ListView backplate；条状元素：ProgressBar、ScrollBar、Slider；**ToolTip（特例，因尺寸小用 4px）** | `ControlCornerRadius`（默认 `4,4,4,4`） |
| **0px** | 直边相接处不圆角（如 SplitButton 的两半之间）；窗口贴边/最大化时不圆角 | — |

两个全局资源 `ControlCornerRadius`、`OverlayCornerRadius` 是可以在 App.xaml 里整体覆盖的。

**Fluent 2（Web）圆角 token（官方 Shapes 页）**

| token | 用途 | 值 |
| --- | --- | --- |
| None | 导航栏、标签栏 | 0 |
| Small | 小 badge | 2px |
| Medium | 按钮、下拉 | 4px |
| Large | 大按钮 | 8px |
| X-Large | button sheet、popover | 12px |
| Circle | 人物头像 | 50% |

官方补充：矩形圆角默认 4px；**小于 32px 的形状圆角降为 2px**；大型与超大型组件用 8px 与 12px。
代码侧 global token：`borderRadiusNone: 0`、`borderRadiusSmall: 2px`、`borderRadiusMedium: 4px`、`borderRadiusLarge: 6px`、`borderRadiusXLarge: 8px`、`borderRadius2XLarge: 12px`、`borderRadius3XLarge: 16px`、`borderRadius4XLarge: 24px`、`borderRadius5XLarge: 32px`、`borderRadius6XLarge: 40px`、`borderRadiusCircular: 10000px`。

**何时不圆角（官方）**：

- 容器内多个元素彼此接触时（例如 SplitButton 的两半）—— 接触处不应有间隙。
- flyout 的单侧与触发它的 UI 相连时（例如 AutoSuggest 的下拉）。
- 组件抵达屏幕边缘时不必圆角。

**AudioLink 取用**：控件 4px、浮层与对话框 8px、卡片 8px、小于 32px 的小元素（如 20px 复选框的方框内部）2px。

### 4.2 Elevation（Windows 11 用「值」表达层级）

Windows 11 用 **elevation value + stroke width** 表达深度。官方值表：

| 表面 | Elevation value | Stroke width |
| --- | --- | --- |
| Window | 128 | 1 |
| Dialog | 128 | 1 |
| Flyout | 32 | 1 |
| Tooltip | 16 | 1 |
| Card | 8 | 1 |
| Control | 2 | 1 |
| Layer | 1 | 1 |

控件状态对 elevation 的影响：Rest = 2 / Hover = 2 / **Pressed = 1**（按下时「贴近」表面）。

阴影与轮廓（contour）共同表达深度：**elevation 越高，阴影越大越柔**。同一 value 下，阴影强度随主题变化。

### 4.3 阴影（Shadow）

**Fluent 2 的 shadow token（官方公式，源码 `shadows.js`）**：

```text
shadow2  = 0 0 2px  {ambient},  0 1px 2px  {key}
shadow4  = 0 0 2px  {ambient},  0 2px 4px  {key}
shadow8  = 0 0 2px  {ambient},  0 4px 8px  {key}
shadow16 = 0 0 2px  {ambient},  0 8px 16px {key}
shadow28 = 0 0 8px  {ambient},  0 14px 28px {key}
shadow64 = 0 0 8px  {ambient},  0 32px 64px {key}
```

代入官方阴影颜色 token（`colorNeutralShadowAmbient` / `colorNeutralShadowKey`）：

| token | Light（ambient / key） | Dark（ambient / key） | 官方用法 |
| --- | --- | --- | --- |
| `shadow2` | rgba(0,0,0,0.12) / rgba(0,0,0,0.14) | rgba(0,0,0,0.24) / rgba(0,0,0,0.28) | 无边框卡片、按下状态的浮动按钮 |
| `shadow4` | 同上 | 同上 | 卡片、网格项、列表项 |
| `shadow8` | 同上 | 同上 | 浮动操作按钮、抬升的卡片与应用栏、命令栏、下拉、tooltip |
| `shadow16` | 同上 | 同上 | callout、hover card |
| `shadow28` | rgba(0,0,0,0.20) / rgba(0,0,0,0.24)（darker 档） | rgba(0,0,0,0.40) / rgba(0,0,0,0.48) | 底部抽屉、侧边导航、抬升的标签栏 |
| `shadow64` | 同上 | 同上 | 弹出对话框、面板 |

官方 elevation 页给的生成公式（低层级）：`Blur = 1 * n`、`X Axis = 0`、`Y Axis = 0.5 * n`、`Opacity = 14%（light）/ 28%（dark，Shadow 1）/ 14%（Shadow 2）`；高层级 ramp 的 Shadow 2 用 `Blur = 8`(light) / `2`(dark)、`X = 0`、`Y = 0`、`Opacity = 20%`。

**关键差异（官方明示）**：`Windows uses strokes instead of key shadows to outline an object.` —— 即 Windows 用 **1px 描边**代替 Fluent 2 的 key shadow。所以 Windows 桌面应用的卡片/控件轮廓应优先用描边（`CardStrokeColorDefault` / `ControlStrokeColorDefault`），阴影只用于真正浮起的表面（flyout、dialog、tooltip）。

**色彩表面上的阴影**：品牌色表面上用亮度方程调整不透明度，否则视觉上达不到同等高度：

```text
Luminosity = 0.2126 * R + 0.7152 * G + 0.0722 * B
Shadow 1 opacity = Round(42 - 0.116 * luminosity)
Shadow 2 opacity = Round(34 - 0.09 * luminosity)
```


---

## 5. 间距与布局（Layout）

### 5.1 间距基准

官方：**base unit = 4px**，4x 系统。global spacing ramp 同时包含 2 / 6 / 10 这三个非 4 倍数值，用于补偿 Fluent 图标的内边距、把图标对齐到 4px 网格。距离从元素的**外框（bounding box）**起算。

Fluent 2 spacing ramp（官方全表）：

| token | 值 | token | 值 | token | 值 | token | 值 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `sizeNone` | 0px | `size20` | 2px | `size40` | 4px | `size60` | 6px |
| `size80` | 8px | `size100` | 10px | `size120` | 12px | `size160` | 16px |
| `size200` | 20px | `size240` | 24px | `size280` | 28px | `size320` | 32px |
| `size360` | 36px | `size400` | 40px | `size480` | 48px | `size520` | 52px |
| `size560` | 56px |  |  |  |  |  |  |

代码侧 global token（Fluent UI v9 `spacings.ts`，共 11 档）：`none=0`、`xxs=2px`、`xs=4px`、`sNudge=6px`、`s=8px`、`mNudge=10px`、`m=12px`、`l=16px`、`xl=20px`、`xxl=24px`、`xxxl=32px`（对应 alias：`spacingHorizontalS/M/L`、`spacingVerticalS/M/L` …）。

触控目标最小尺寸（官方）：iOS 与 Web **44 × 44**，Android **48 × 48**。

### 5.2 栅格

- 栅格三要素：**columns / gutters / margins**。
- 常用 **12 列**框架（可等分为 halves / thirds / fourths / sixths），随屏幕宽度响应式变化。
- gutter 宽度必须是基准单位的倍数，可在不同断点变化。
- margin 是栅格外的留白，可固定或按百分比，也可随断点变化。
- 基线栅格（dense horizontal rows）用于建立垂直节奏，适合跨多列的内容。
- 官方未在 layout 页给出「内容最大宽度」的固定 px 值；**未找到权威值**。AudioLink 是桌面应用，建议以窗口宽度为准，正文类内容限制在 50–60 字符行宽（见 3.2）。

### 5.3 控件高度与尺寸（WinUI 3 源码口径）

| 控件 | 官方值 | 来源 |
| --- | --- | --- |
| Button — padding | `ButtonPadding = 11,5,11,6` | `Button_themeresources.xaml` |
| Button — 边框 | `ButtonBorderThemeThickness = 1` | 同上 |
| Button — 圆角 | `ControlCornerRadius = 4` | 同上 |
| Button — 字号 | `ControlContentThemeFontSize`（14px） | 同上 |
| Button — 窄按钮下限 | 官方建议 **最小宽度 120px**（短文本避免窄按钮） | WinUI Buttons 页 |
| Button — 文本长度 | > 26 字符时：加宽或堆叠换行，或加高换行 | 同上 |
| TextBox — padding | `TextControlThemePadding = 10,5,6,6` | `Common_themeresources.xaml` |
| TextBox — 边框 | `TextControlBorderThemeThickness = 1` | 同上 |
| TextBox — 聚焦边框 | `TextControlBorderThemeThicknessFocused = 1,1,1,2` | 同上 |
| TextBox — 字号 | `ControlContentThemeFontSize`（14px） | `TextBox_themeresources.xaml` |
| TextBox — 内部图标字号 | `TextBoxIconFontSize = 12` | 同上 |
| CheckBox — 方框 | `CheckBoxSize = 20`，勾选符 `CheckBoxGlyphSize = 12`，边框 `CheckBoxBorderThickness = 1` | `CheckBox_themeresources.xaml` |
| CheckBox — 尺寸 | `CheckBoxHeight = 32`（MinHeight），`CheckBoxMinWidth = 120` | 同上 |
| ToggleSwitch — 尺寸 | `ToggleSwitchThemeMinWidth = 154`，前/后内容外边距各 10 | `ToggleSwitch_themeresources.xaml` |
| ToggleSwitch — 描边 | `ToggleSwitchOuterBorderStrokeThickness = 1` | 同上 |
| Slider — 轨道 | `SliderTrackThemeHeight = 4`（另有 2 的变体），`SliderOutsideTickBarThemeHeight = 4` | `Slider_themeresources.xaml` |
| Slider — 滑块 | `SliderHorizontalThumbWidth/Height = 18`，内圈 `SliderInnerThumbWidth/Height = 12` | 同上 |
| Slider — 容器 | `SliderHorizontalHeight = 32`，`SliderVerticalWidth = 32`，前后内容外边距各 14 | 同上 |
| NavigationView — header | 固定高度 **52px**，与左侧导航按钮垂直对齐；在 Minimal 模式下始终可见 | WinUI NavigationView 页 |
| NavigationView — 内容边距 | Minimal 模式 **12px**，其它模式 **24px** | 同上 |

**未找到权威值**：Button / TextBox 的默认 **MinHeight = 32px**（WinUI 的 32px 标准控件高度）未在上述开源主题资源里出现（由框架默认样式提供）。官方 Fluent UI v9 的控件尺寸档位也是 Small / Medium(32px) / Large(40px) 的一致口径，但与 WinUI 源码的对应关系未在本次抓取中直接验证 —— 落地时按 32px 实现并在实机上比对。

### 5.4 NavigationView 布局规格

| 项目 | 值 |
| --- | --- |
| 导航样式 | top / left（`PaneDisplayMode`：`Auto`、`Left`、`LeftCompact`、`LeftMinimal`、`Top`） |
| 自动断点 | ≥ **1008px** → `Left`（展开左栏）；**641–1007px** → `LeftCompact`（仅图标）；≤ **640px** → `LeftMinimal`（只有一个菜单按钮） |
| 断点属性 | `CompactModeThresholdWidth`（默认 640）、`ExpandedModeThresholdWidth`（默认 1008） |
| header | 固定 52px；Left 模式下与导航按钮对齐，Top 模式下位于面板下方；是内容区的滚动裁剪点 |
| 内容边距 | Minimal 12px，其它 24px |
| 层级 | Left/Top 模式默认自带 content layer；要改用 card pattern 必须先覆盖背景与边框主题资源 |

### 5.5 图标尺寸

| 尺寸 | 何时用 | 来源/口径 |
| --- | --- | --- |
| **12px** | 仅用于**传达信息/补充概念**，**太小不能用于交互** | Fluent 2 Iconography 页原文（Use 12 pixel icons to give information … generally too small for interactions） |
| **16px** | 紧凑列表项、行内图标、状态指示 | WinUI 控件内图标基准（`IconSource` / `FontIcon` 常用 16） |
| **20px** | 标准命令按钮内图标、NavigationView 图标 | NavigationView 默认图标尺寸 20 |
| **24px** | 工具栏/标题栏图标、更大的点击目标 | 常用主体尺寸 |
| **32px+** | 空状态插画、设置项分组图标 | — |

图标主题（官方）：**Regular** 用于识别与选择动作（wayfinding）；**Filled** 用于**选中态高亮**或需要更多视觉重量的小图标。

图标其它规则：

- 命名按**形状/物体**而非功能（Shield, not security）。
- 修饰符（modifier）永远用 filled 主题，且固定放在图标**右下角**。
- 只用**单色**；系统图标可改色，产品启动图标**永远不能改色**。
- 产品图标缩放到 48px 以下时会简化细节；放大超过 48px 时按 4 的倍数缩放（48/64/96…）。

16 / 20 / 24 / 32 的具体使用场景在 Fluent 2 Iconography 页**没有逐条规定**（官方只给了 12px 与产品图标的规则）；上表中 16/20/24 属于 WinUI 控件默认值口径，**以控件实测为准**。


---

## 6. 控件规格

### 6.1 按钮

三种官方样式的**填充与边框规则**（来自 `Button_themeresources.xaml`）：

| 样式 | 槽位 | 默认 | Hover | Pressed | 边框 |
| --- | --- | --- | --- | --- | --- |
| **Standard（默认 `DefaultButtonStyle`）** | `ButtonBackground` | `ControlFillColorDefaultBrush` | `ControlFillColorSecondaryBrush` | `ControlFillColorTertiaryBrush` | `ButtonBorderBrush = ControlElevationBorderBrush`（渐变），pressed 时退为 `ControlStrokeColorDefaultBrush` |
| **Accent（`AccentButtonStyle`）** | `AccentButtonBackground` | accent fill（= `SystemAccentColor`） | accent hover（accent + 90% 不透明） | accent pressed（accent + 80% 不透明） | `AccentButtonBorderBrush`；`BackgroundSizing = OuterBorderEdge` |
| **Subtle（`SubtleButtonStyle`）** | `SubtleButtonBackground` | `SubtleFillColorTransparentBrush`（无底） | `SubtleFillColorSecondaryBrush` | `SubtleFillColorTertiaryBrush` | `SubtleButtonBorderBrush`（透明）；`BackgroundSizing = InnerBorderEdge` |

共用规格：`Padding = 11,5,11,6`、`BorderThickness = 1`、`CornerRadius = ControlCornerRadius(4)`、`FontSize = 14`、`FontWeight = Normal`、`FocusVisualMargin = -3`。

**控件底边的官方实现（重要识别特征）**：standard 按钮的边框不是纯色，而是垂直渐变：

```xml
<LinearGradientBrush x:Key=ControlElevationBorderBrush MappingMode=Absolute StartPoint=0,0 EndPoint=0,3>
  <GradientStop Offset=0.33 Color={StaticResource ControlStrokeColorSecondary} />
  <GradientStop Offset=1.0  Color={StaticResource ControlStrokeColorDefault} />
</LinearGradientBrush>
```

即：按钮边框上部取更深的 `ControlStrokeColorSecondary`（Light `#29000000` / Dark `#18FFFFFF`），下部取更浅的 `ControlStrokeColorDefault`（Light `#0F000000` / Dark `#12FFFFFF`）。

### 6.2 文本框（TextBox）—— 特别注意底部强调线

官方规则：

1. **默认/未聚焦边框**：`BorderThickness = 1`，`BorderBrush = TextControlElevationBorderBrush` —— 同样是垂直渐变，但带 `ScaleTransform ScaleY=-1 CenterY=0.5`，`StartPoint=0,0 → EndPoint=0,2`，色标为：

```xml
offset 0.5 -> ControlStrongStrokeColorDefault   (Light #72000000 / Dark #8BFFFFFF)
offset 1.0 -> ControlStrokeColorDefault         (Light #0F000000 / Dark #12FFFFFF)
```

效果：**输入控件的下边界明显比上边界深** —— 这就是「输入控件有一条更深的底边」的官方来源。

2. **聚焦态**：

- `TextControlBorderThemeThicknessFocused = 1,1,1,2` —— **底边从 1px 加粗到 2px**（HighContrast 下整体 2px）。
- 边框刷换成 `TextControlElevationBorderFocusedBrush`，其色标为 `SystemAccentColorLight2`（accent 的 Light2 派生色）。
- 背景换成 `TextControlBackgroundFocused = ControlFillColorInputActiveBrush`（Light `#FFFFFF` 纯白 / Dark `#B31E1E1E`）。

3. 其它状态：

| 状态 | Background | BorderBrush | Placeholder 前景 |
| --- | --- | --- | --- |
| Rest | `ControlFillColorDefaultBrush` | `TextControlElevationBorderBrush` | `TextFillColorSecondaryBrush` |
| PointerOver | `ControlFillColorSecondaryBrush` | `TextControlElevationBorderBrush`（不变） | `TextFillColorSecondaryBrush` |
| Focused | `ControlFillColorInputActiveBrush` | `TextControlElevationBorderFocusedBrush`（accent） | `TextFillColorSecondaryBrush` |
| Disabled | — | `ControlStrokeColorDefaultBrush` | `TextFillColorDisabledBrush` |

4. 内边距 `TextControlThemePadding = 10,5,6,6`；圆角 `ControlCornerRadius`(4)。

> FDS 落地警示：WinUI 的「底部强调线」不是 CSS `border-bottom: 2px` 就能等价还原的 —— 未聚焦时它是一条**从上到下渐变的 1px 边框**。CSS 侧建议用两层实现：外层 1px 全边框（`box-shadow: inset 0 0 0 1px` 或用 border），再叠一条 `border-bottom-color` 更深的伪元素线；聚焦时把底部改为 2px accent 线并把背景提升为 `ControlFillColorInputActive`。

### 6.3 复选框（CheckBox）

- 方框 `20 × 20`，勾选符 `12 × 12`，边框 1px，圆角 4px（`ControlCornerRadius`；官方对小于 32px 的形状允许 2px）。
- 控件尺寸：`MinHeight = 32`，`MinWidth = 120`。
- 选中态填充使用 **compound brand** 语义（`colorCompoundBrandBackground` / WinUI 侧 accent fill），而不是普通 brand background —— 因为复合控件（复选框/开关/滑块）使用独立的 compound 色以适配暗色。

### 6.4 开关（ToggleSwitch）

- `MinWidth = 154`（含 label），前后内容外边距各 10。
- 外框描边厚度 1（`ToggleSwitchOuterBorderStrokeThickness = 1`），胶囊形（pill）轨道 —— 官方 Shapes 页明确 **pill 用于 toggle 的 channel 与 slider 的 track**。
- 圆角：整控件 `ControlCornerRadius`，但轨道本身是 pill（`borderRadiusCircular` 口径）。

### 6.5 滑块（Slider）

- 轨道高 4px（另有 2px 变体用于紧凑场景），刻度条高 4px。
- 滑块（thumb）：18 × 18，内圈 12 × 12（悬停/拖动时的内圈反馈）。
- 容器高/宽（水平/垂直）32px，前后内容外边距各 14。
- 轨道圆角使用 `SliderTrackCornerRadius`（pill）。

### 6.6 选项卡与导航视图（TabView / NavigationView）

**NavigationView 结构（官方）**：

```text
┌────────────────────────────────────────────────────┐
│ 标题栏（Mica 露出；Left 模式下导航按钮与 header 对齐）  │
├──────────────┬─────────────────────────────────────┤
│ NavigationViewPane │  Header（固定 52px，页面标题）      │
│  · 主导航项      ├─────────────────────────────────────┤
│  · 分隔符        │  Content（Minimal 模式 12px，其它 24px 边距）│
│  · 底部项（设置） │                                     │
└──────────────┴─────────────────────────────────────┘
```

- 模式：`Auto`（默认，按宽度自动切 `Left` / `LeftCompact` / `LeftMinimal`）、`Left`、`Top`、`LeftCompact`、`LeftMinimal`。
- 断点：1008px / 640px（见 5.4）。
- Top 模式下项目随窗口变窄收进 overflow 菜单；此时更好的做法是整体切到 `LeftMinimal`，而不是让所有项挤进 overflow。
- 层级：Left/Top 默认在内容区自带 content layer；要改成 card pattern 需覆盖默认背景与边框主题资源。

**TabView**：本次抓取中 `TabView_themeresources.xaml` 未能取到，**TabView 的具体尺寸（标签高度、内边距）未找到权威值**。可确认的官方事实：
TabView 的典型存在形式就是「带选项卡的标题栏」，而官方明确说这种场景应当用 **Mica Alt** 作为底（Because Mica Alt provides a deeper visual hierarchy … especially when creating an app with a tabbed title bar）。

### 6.7 焦点视觉（Focus）

- WinUI 用**系统焦点视觉**（`UseSystemFocusVisuals = True`），由两层构成：外圈 `FocusStrokeColorOuter`（Light `#E4000000` / Dark `#FFFFFF`）+ 内圈 `FocusStrokeColorInner`（Light `#B3FFFFFF` / Dark `#B3000000`）。
- Fluent 2 alias：`colorStrokeFocus1`（Light `#ffffff` / Dark `#000000`）、`colorStrokeFocus2`（Light `#000000` / Dark `#ffffff`）。
- 按钮的 `FocusVisualMargin = -3`（焦点框向外扩 3px）。
- 官方交互状态规则：**focus 不改变控件颜色，只加粗描边**，用于区分鼠标与键盘交互。


---

## 7. 动效（Motion）

### 7.1 时长 token（Fluent 2 官方 `durations.ts`）

| token | 值 | 建议用途（AudioLink） |
| --- | --- | --- |
| `durationUltraFast` | 50ms | 颜色/描边的微反馈 |
| `durationFaster` | 100ms | hover 状态、图标切换 |
| `durationFast` | 150ms | 按钮按下、复选框勾选 |
| `durationNormal` | 200ms | **默认时长**：flyout 进出、卡片展开 |
| `durationGentle` | 250ms | 轻微位移的容器变化 |
| `durationSlow` | 300ms | 面板滑出、NavigationView 展开 |
| `durationSlower` | 400ms | 大面积容器变换 |
| `durationUltraSlow` | 500ms | 极少用（全屏过渡） |

### 7.2 缓动曲线 token（官方 `curves.ts`）

| token | cubic-bezier | 语义 |
| --- | --- | --- |
| `curveAccelerateMax` | `cubic-bezier(0.9,0.1,1,0.2)` | 最大加速（元素离场） |
| `curveAccelerateMid` | `cubic-bezier(1,0,1,1)` | 中等加速 |
| `curveAccelerateMin` | `cubic-bezier(0.8,0,0.78,1)` | 最小加速 |
| `curveDecelerateMax` | `cubic-bezier(0.1,0.9,0.2,1)` | 最大减速（元素入场） |
| `curveDecelerateMid` | `cubic-bezier(0,0,0,1)` | 中等减速 |
| `curveDecelerateMin` | `cubic-bezier(0.33,0,0.1,1)` | 最小减速 |
| `curveEasyEaseMax` | `cubic-bezier(0.8,0,0.2,1)` | 最大缓入缓出 |
| `curveEasyEase` | `cubic-bezier(0.33,0,0.67,1)` | **标准缓入缓出** |
| `curveLinear` | `cubic-bezier(0,0,1,1)` | 线性（**仅用于旋转等需要恒定速率**的场景） |

### 7.3 四条动效原则（官方）

| 原则 | 含义 |
| --- | --- |
| **Functional** | 有目的有意图地动：指示下一步、告知 UI 变化、庆祝完成 |
| **Natural** | 遵循物理规律（惯性、重力、重量、速度），让动画可信可预测 |
| **Consistent** | 跨产品一致的动效带来熟悉感 |
| **Appealing** | 恰当的小惊喜 |

### 7.4 什么该动、什么不该动

官方四种主要过渡：

1. **Enter and exit** —— 元素的引入与消失（菜单、对话框等交互元素的出现/消失）。
2. **Elevation** —— 表达高度或深度的变化（按钮状态、拖放、窗口、层级）。
3. **Top level** —— 在不同页面/目的地之间导航。官方明确：**顶层元素面积大，应使用快速淡入淡出，不要把 UI 元素滑进滑出**，否则会产生非预期的层级与迷失感。
4. **Container transform** —— 容器布局/位置的变化（resize / reposition，响应式布局调整）。

编排（Choreography）：

- **Staggering**：延迟多个动画的开始，用于软化大量条目的入场，或引导视线方向。用**短偏移**；同一内容多次加载时可程序化地变化偏移（先大后小）。
- **重要元素**给更明显的位移与更长时长；**次要元素**应当**同步**计时、成组处理，形成整体感。
- 官方总结：Keep durations short and the movement natural.（时长要短，运动要自然。）

不该动的：

- 顶层页面切换不要做位移/滑屏（用淡入淡出）。
- 连续的、无目的的装饰动画（这属于反模式，见第 9 节）。


---

## 8. 识别特征清单：什么让界面一眼看出是 Fluent

| # | 特征 | 可执行判据 |
| --- | --- | --- |
| 1 | **窗口底是 Mica，不是纯色** | 窗口背景使用系统 backdrop（Mica / Mica Alt）；失焦时自动转中性色（不能自己在 CSS 里画壁纸色） |
| 2 | **两层分层** | 底（base）+ 内容层（`LayerFillColorDefault = rgba(255,255,255,0.502)` Light / `rgba(58,58,58,0.298)` Dark）或卡片层，内容层永远是低不透明度纯色而非渐变 |
| 3 | **控件圆角 4px、浮层 8px** | Button / TextBox / CheckBox / ListView item = 4px；Flyout / Dialog / MenuFlyout / TeachingTip = 8px；ToolTip = 4px |
| 4 | **控件只有 1px 描边，且描边是渐变的** | 按钮边框用 `ControlElevationBorderBrush`（上 `ControlStrokeColorSecondary` → 下 `ControlStrokeColorDefault`）；不是四边同色的 `1px solid` |
| 5 | **输入控件有一条更深的底边** | TextBox 未聚焦边框 = 垂直渐变（下部 `ControlStrongStrokeColorDefault`）；聚焦 = **底边 2px + accent 色** |
| 6 | **浮出层用 Acrylic（磨砂玻璃），长驻内容绝不用** | Flyout / ContextMenu / ComboBox 下拉 = acrylic；页面、列表、导航面板 = 不透明或低透明度纯色 |
| 7 | **模态用 Smoke 遮罩** | 对话框背后 `rgba(0,0,0,0.302)`，light/dark 同值 |
| 8 | **焦点是两层描边，不是 outline** | 内圈白 + 外圈黑（Dark 下反转），按钮焦点框向外扩 3px |
| 9 | **交互越深越「亮」（Windows 口径）** | Windows 上控件 hover/pressed 比 rest 更亮/更透（与 Fluent 2 web 的「越交互越深」相反） |
| 10 | **字重只用 Regular / Semibold** | 标题 Semibold，正文 Regular，字号只有 12/14/18/20/28/40/68 这几档，行高 16/20/24/28/36/52/92 |
| 11 | **强调色只出现在小面积** | accent 用于主按钮、开关/复选框选中、滑块轨道、链接、焦点描边；不做大面积色块、不做渐变文字 |
| 12 | **图标用 Fluent System Icons，Regular/Filled 双主题** | 常规态 Regular；选中态 Filled；单色；修饰符在右下角 |


---

## 9. 反模式（Fluent 明确不用什么）

| 反模式 | 官方依据 |
| --- | --- |
| **大圆角胶囊按钮**（pill button） | 圆角体系里按钮 = 4px（大按钮 8px），pill 只用于 toggle channel / slider track / tag / avatar；Fluent 2 圆角表里按钮没有 50% 档 |
| **大面积毛玻璃装饰** | 官方明示：Don't put desktop acrylic on large background surfaces；Acrylic is GPU-intensive |
| **并排多块 acrylic / 多层 acrylic 叠加** | Don't place multiple acrylic panes next to each other（会产生可见接缝）；多层 background acrylic 会产生令人分心的视错觉 |
| **acrylic 上放 accent 色文字或超链接** | 官方：这些组合在默认 14px 字号下大概率无法通过最低对比度要求 |
| **把 backdrop 材质套在某个 UI 元素上** | Don't apply backdrop material to a UI element（backdrop 只在背后层级全透明时才可见） |
| **一个应用里叠加多层 backdrop** | Don't apply backdrop material more than once in an application |
| **用彩色渐变文字/彩虹渐变装饰** | 调色板规则要求中性色承载 surface 与文字；语义色不得用于装饰；品牌色避免大面积与过度使用 |
| **把语义色当装饰用** | 官方：Use semantic colors for important messages / Don't use them for decoration |
| **Bold 与 Italic 做强调** | Bold 与 Italic 不属于 Windows type ramp；用 Semibold；Italic 会降低可读性（尤其阅读障碍用户） |
| **ALL CAPS / 全大写标题** | 官方：Don't use all caps to get a user's attention |
| **顶层页面切换用滑动/位移动画** | 官方 Top level 过渡明确要求用快速淡入淡出，不要移动大块 UI |
| **卡片上同时用阴影和边框去堆叠高度** | Windows 用 stroke 代替 key shadow；阴影只用于真正浮起的表面，Overusing shadows … can create visual noise |
| **焦点用系统默认 outline / 用颜色变化表达 focus** | 官方：focus 态不改变控件颜色，而是容器获得更粗的描边 |


---

## 10. 给 Tauri / React 的落地映射

### 10.1 CSS 变量命名约定（Tailwind v4 `@theme`）

命名规则：`--al-<domain>-<role>-<state>`。两层：`--al-global-*`（原始值，只写一次）→ `--al-*`（语义 alias，界面只准用这一层）。

```css
@import 'tailwindcss';

/* ===================== Layer 1: global（原始值，永不直接使用） ===================== */
:root {
  /* neutral ramp（Fluent 2 grey，节选常用档） */
  --al-global-grey-8: #141414;   --al-global-grey-10: #1a1a1a;
  --al-global-grey-12: #1f1f1f;  --al-global-grey-14: #242424;
  --al-global-grey-16: #292929;  --al-global-grey-24: #3d3d3d;
  --al-global-grey-26: #424242;  --al-global-grey-38: #616161;
  --al-global-grey-40: #666666;  --al-global-grey-68: #adadad;
  --al-global-grey-82: #d1d1d1;  --al-global-grey-88: #e0e0e0;
  --al-global-grey-94: #f0f0f0;  --al-global-grey-96: #f5f5f5;
  --al-global-grey-98: #fafafa;  --al-global-white: #ffffff;
  --al-global-black: #000000;
  /* brand ramp（brandWeb 关键档） */
  --al-global-brand-40: #0c3b5e; --al-global-brand-60: #0f548c;
  --al-global-brand-70: #115ea3; --al-global-brand-80: #0f6cbd;
  --al-global-brand-100: #479ef5; --al-global-brand-110: #62abf5;
  /* 系统 accent（运行时会由 Tauri 注入覆盖） */
  --al-global-accent: #0078d4;
  /* 圆角 / 描边 / 时长 / 曲线 */
  --al-global-radius-control: 4px;
  --al-global-radius-overlay: 8px;
  --al-global-radius-small: 2px;
  --al-global-stroke-thin: 1px;
  --al-global-stroke-thick: 2px;
  --al-global-duration-fast: 150ms;
  --al-global-duration-normal: 200ms;
  --al-global-duration-slow: 300ms;
  --al-global-ease-standard: cubic-bezier(0.33, 0, 0.67, 1);
  --al-global-ease-decelerate: cubic-bezier(0.1, 0.9, 0.2, 1);
  --al-global-ease-accelerate: cubic-bezier(0.9, 0.1, 1, 0.2);
}

/* ===================== Layer 2: alias（界面只准用这一层） ===================== */
:root {
  /* 表面：对应 WinUI LayerFillColorDefault / SolidBackgroundFillColorBase 等 */
  --al-surface-base: #f3f3f3;              /* SolidBackgroundFillColorBase */
  --al-surface-base-alt: #dadada;          /* SolidBackgroundFillColorBaseAlt */
  --al-surface-layer: rgba(255,255,255,0.502);   /* LayerFillColorDefault */
  --al-surface-card: rgba(255,255,255,0.702);    /* CardBackgroundFillColorDefault */
  --al-surface-card-stroke: rgba(0,0,0,0.059);   /* CardStrokeColorDefault */
  --al-surface-control: rgba(255,255,255,0.702); /* ControlFillColorDefault */
  --al-surface-control-hover: rgba(249,249,249,0.502);
  --al-surface-control-pressed: rgba(249,249,249,0.302);
  --al-surface-control-disabled: rgba(249,249,249,0.302);
  --al-surface-input-active: #ffffff;            /* ControlFillColorInputActive */
  --al-surface-flyout: rgba(249,249,249,0.702);  /* acrylic 降级底色（图层） */
  --al-surface-subtle-hover: rgba(0,0,0,0.035);  /* SubtleFillColorSecondary */
  --al-surface-subtle-pressed: rgba(0,0,0,0.024);/* SubtleFillColorTertiary */
  --al-surface-smoke: rgba(0,0,0,0.302);         /* SmokeFillColorDefault */
  /* 文字 */
  --al-text-primary: rgba(0,0,0,0.894);   /* TextFillColorPrimary */
  --al-text-secondary: rgba(0,0,0,0.62);  /* TextFillColorSecondary */
  --al-text-tertiary: rgba(0,0,0,0.447);  /* TextFillColorTertiary */
  --al-text-disabled: rgba(0,0,0,0.361);  /* TextFillColorDisabled */
  --al-text-on-accent: #ffffff;           /* TextOnAccentFillColorPrimary */
  /* 描边 */
  --al-stroke-control: rgba(0,0,0,0.059);        /* ControlStrokeColorDefault */
  --al-stroke-control-strong: rgba(0,0,0,0.447);  /* ControlStrongStrokeColorDefault */
  --al-stroke-control-secondary: rgba(0,0,0,0.161);/* ControlStrokeColorSecondary */
  --al-stroke-divider: rgba(0,0,0,0.059);         /* DividerStrokeColorDefault */
  --al-stroke-focus-outer: rgba(0,0,0,0.894);     /* FocusStrokeColorOuter */
  --al-stroke-focus-inner: rgba(255,255,255,0.702);/* FocusStrokeColorInner */
  /* accent / brand */
  --al-accent: var(--al-global-accent);            /* AccentFillColorDefault = SystemAccentColor */
  --al-accent-hover: var(--al-global-brand-70);    /* 回退口径 */
  --al-accent-pressed: var(--al-global-brand-40);  /* 回退口径 */
  --al-accent-fg: #ffffff;
  /* 语义色 */
  --al-success: #0f7b0f;   --al-success-bg: #dff6dd;
  --al-caution: #9d5d00;   --al-caution-bg: #fff4ce;
  --al-critical: #c42b1c;  --al-critical-bg: #fde7e9;
  /* 形状 / 层级 */
  --al-radius-control: var(--al-global-radius-control);
  --al-radius-overlay: var(--al-global-radius-overlay);
  --al-shadow-2: 0 0 2px rgba(0,0,0,0.12), 0 1px 2px rgba(0,0,0,0.14);
  --al-shadow-8: 0 0 2px rgba(0,0,0,0.12), 0 4px 8px rgba(0,0,0,0.14);
  --al-shadow-16: 0 0 2px rgba(0,0,0,0.12), 0 8px 16px rgba(0,0,0,0.14);
  --al-shadow-28: 0 0 8px rgba(0,0,0,0.20), 0 14px 28px rgba(0,0,0,0.24);
  --al-shadow-64: 0 0 8px rgba(0,0,0,0.20), 0 32px 64px rgba(0,0,0,0.24);
}

/* ===================== Dark 主题（对齐 WinUI Default 主题字典） ===================== */
@media (prefers-color-scheme: dark) {
  :root {
    --al-surface-base: #202020;
    --al-surface-base-alt: #0a0a0a;
    --al-surface-layer: rgba(58,58,58,0.298);
    --al-surface-card: rgba(255,255,255,0.051);
    --al-surface-card-stroke: rgba(0,0,0,0.098);
    --al-surface-control: rgba(255,255,255,0.059);
    --al-surface-control-hover: rgba(255,255,255,0.082);
    --al-surface-control-pressed: rgba(255,255,255,0.031);
    --al-surface-input-active: rgba(30,30,30,0.702);
    --al-surface-subtle-hover: rgba(255,255,255,0.059);
    --al-surface-subtle-pressed: rgba(255,255,255,0.039);
    --al-text-primary: #ffffff;
    --al-text-secondary: rgba(255,255,255,0.773);
    --al-text-tertiary: rgba(255,255,255,0.529);
    --al-text-disabled: rgba(255,255,255,0.365);
    --al-stroke-control: rgba(255,255,255,0.071);
    --al-stroke-control-strong: rgba(255,255,255,0.545);
    --al-stroke-control-secondary: rgba(255,255,255,0.094);
    --al-stroke-divider: rgba(255,255,255,0.082);
    --al-stroke-focus-outer: #ffffff;
    --al-stroke-focus-inner: rgba(0,0,0,0.702);
    --al-accent: var(--al-global-brand-100);
    --al-accent-hover: var(--al-global-brand-110);
    --al-accent-pressed: var(--al-global-brand-40);
    --al-success: #6ccb5f;  --al-success-bg: #393d1b;
    --al-caution: #fce100;  --al-caution-bg: #433519;
    --al-critical: #ff99a4; --al-critical-bg: #442726;
    --al-shadow-2: 0 0 2px rgba(0,0,0,0.24), 0 1px 2px rgba(0,0,0,0.28);
    --al-shadow-8: 0 0 2px rgba(0,0,0,0.24), 0 4px 8px rgba(0,0,0,0.28);
    --al-shadow-16: 0 0 2px rgba(0,0,0,0.24), 0 8px 16px rgba(0,0,0,0.28);
  }
}
```

### 10.2 Tailwind v4 `@theme` 暴露（生成工具类）

```css
@theme {
  --color-surface-base: var(--al-surface-base);
  --color-surface-layer: var(--al-surface-layer);
  --color-surface-card: var(--al-surface-card);
  --color-surface-control: var(--al-surface-control);
  --color-text-primary: var(--al-text-primary);
  --color-text-secondary: var(--al-text-secondary);
  --color-stroke-control: var(--al-stroke-control);
  --color-accent: var(--al-accent);
  --color-success: var(--al-success);
  --color-caution: var(--al-caution);
  --color-critical: var(--al-critical);
  --radius-control: var(--al-radius-control);   /* -> rounded-control */
  --radius-overlay: var(--al-radius-overlay);   /* -> rounded-overlay */
  --text-caption: 12px;   --text-caption--line-height: 16px;
  --text-body: 14px;      --text-body--line-height: 20px;
  --text-body-lg: 18px;   --text-body-lg--line-height: 24px;
  --text-subtitle: 20px;  --text-subtitle--line-height: 28px;
  --text-title: 28px;     --text-title--line-height: 36px;
  --text-title-lg: 40px;  --text-title-lg--line-height: 52px;
  --font-ui: 'Segoe UI Variable Text', 'Segoe UI Variable', 'Segoe UI', system-ui, sans-serif;
  --font-display: 'Segoe UI Variable Display', 'Segoe UI Variable', 'Segoe UI', sans-serif;
  --ease-fluent: var(--al-global-ease-standard);
  --ease-fluent-in: var(--al-global-ease-accelerate);
  --ease-fluent-out: var(--al-global-ease-decelerate);
}
```

得到 `bg-surface-card`、`text-text-secondary`、`border-stroke-control`、`rounded-control`、`text-body`、`ease-fluent` 等工具类。

### 10.3 工具类 / 组件约定（照抄清单）

| 场景 | 约定 |
| --- | --- |
| 窗口底 | 不要用 CSS 画 Mica。Tauri 侧用窗口材质（`window-vibrancy` 的 `apply_mica` / `apply_acrylic`，或 Win32 `DwmSetWindowAttribute`），前端把 `html, body, #root` 背景设为 `transparent` |
| 内容层 | `.al-layer { background: var(--al-surface-layer); }` —— 只在窗口底为 Mica 时使用 |
| 卡片 | `.al-card { background: var(--al-surface-card); border: 1px solid var(--al-surface-card-stroke); border-radius: var(--al-radius-overlay); }`（Windows 用描边代替 key shadow） |
| 按钮（standard） | `border: 1px solid var(--al-stroke-control); background: var(--al-surface-control); border-radius: var(--al-radius-control); padding: 5px 11px 6px; font-size: 14px; font-weight: 400; min-height: 32px;` |
| 按钮（accent） | `background: var(--al-accent); color: var(--al-text-on-accent); border: 1px solid transparent;`（hover/pressed 走 `--al-accent-hover` / `--al-accent-pressed`） |
| 按钮（subtle） | 默认 `background: transparent; border-color: transparent;` → hover `var(--al-surface-subtle-hover)` → pressed `var(--al-surface-subtle-pressed)` |
| 输入框 | `border: 1px solid var(--al-stroke-control-strong); border-bottom-color: var(--al-stroke-control-strong); border-radius: var(--al-radius-control); padding: 5px 10px 6px;`；`+ :focus` → `border-bottom-width: 2px; border-bottom-color: var(--al-accent); background: var(--al-surface-input-active);` |
| 浮出层 | `background: var(--al-surface-flyout); backdrop-filter: blur(30px) saturate(120%); border: 1px solid var(--al-surface-card-stroke); border-radius: var(--al-radius-overlay); box-shadow: var(--al-shadow-8);`（**仅限 flyout / menu / popover**） |
| 对话框遮罩 | `background: var(--al-surface-smoke);` |
| 焦点环 | `.al-focusable:focus-visible { outline: 2px solid var(--al-stroke-focus-outer); outline-offset: 1px; box-shadow: 0 0 0 1px var(--al-stroke-focus-inner); }` |
| 分隔线 | `1px solid var(--al-stroke-divider)` |
| 动效 | `transition: background-color var(--al-global-duration-fast) var(--al-global-ease-standard), border-color var(--al-global-duration-fast) var(--al-global-ease-standard);` |
| 减少动效 | `@media (prefers-reduced-motion: reduce) { *, *::before, *::after { animation-duration: .01ms !important; transition-duration: .01ms !important; } }` |
| 字号纪律 | 只用 12 / 14 / 18 / 20 / 28 / 40 档；强调用 `font-weight: 600`；不用 500 以下的标题、不用 italic |
| 圆角纪律 | 禁止 `rounded-full` 用在按钮上；pill 只用于开关轨道、滑块轨道、tag、头像 |
| 图标 | `@fluentui/react-icons`；常规 Regular，选中/强调 Filled；尺寸 16 / 20 / 24（12px 仅信息性图标，不可交互） |
| 系统 accent | Tauri 命令读取系统 accent（Win32 `DwmGetColorizationColor` 或注册表 `HKCU\Software\Microsoft\Windows\DWM\AccentColor`）→ 写入 `--al-accent`；失败回退 `#0078d4` |
| 主题 | 跟随 `prefers-color-scheme`，并提供手动覆盖开关（对齐系统设置） |

### 10.4 Tauri 特有的注意点

1. **Mica/Acrylic 必须由窗口层实现**，CSS `backdrop-filter` 只能处理「窗口内元素之间的模糊」，拿不到桌面壁纸。两个方案：
   - 用 `window-vibrancy` crate：`apply_mica(&window, Some(true))` / `apply_mica(&window, None)`（Alt）/ `apply_acrylic(...)`；
   - 或 Win32：`DwmSetWindowAttribute(DWMWA_SYSTEMBACKDROP_TYPE, DWMSBT_MAINWINDOW / DWMSBT_TABBEDWINDOW / DWMSBT_TRANSIENTWINDOW)`。
2. **必须实现降级**：系统关闭透明效果、进入节电模式时，Mica 会失效 → 前端不要把「底是 Mica」当作布局前提，`--al-surface-base` 要始终作为可用底色。
3. **圆角**：窗口本身的 8px 圆角由 DWM 提供（Windows 11），WebView 内部不要重复画窗口级圆角；只在内部容器上用 4px / 8px。
4. **字体**：WebView2 (Chromium) 支持可变字体，`Segoe UI Variable Text/Display` 可直接用；但为避免 opsz 未按预期生效，显式声明 `font-variation-settings` 是可选优化，默认建议让它自动匹配。
5. **动画**：WebView 的 `transition` 与 Windows 原生动效不完全一致，时长/曲线按第 7 节 token 取值即可，不需要复刻系统 spring。

### 10.5 落地自检清单

- [ ] 窗口底是 Mica / Mica Alt，失焦时自动变化，关透明效果时自动退回纯色
- [ ] 长驻内容**没有**用 acrylic；acrylic 只出现在 flyout / menu / popover
- [ ] 控件圆角 4px、浮层 8px、≤32px 的小元素 2px；没有一个 pill 按钮
- [ ] TextBox 未聚焦有明显更深的底边，聚焦时底边 2px accent
- [ ] 焦点态用双层描边，未用颜色变化表达 focus
- [ ] 字号只取 12/14/18/20/28/40；强调只用 Semibold；无 italic、无全大写
- [ ] accent 使用面积小且有节制；语义色只用于状态，不做装饰
- [ ] 顶层页面切换是淡入淡出，不是滑动
- [ ] 阴影只出现在浮起的表面，其余用 1px 描边


---

## 附：来源清单

**Fluent 2 官网（fluent2.microsoft.design）**

1. https://fluent2.microsoft.design/material
2. https://fluent2.microsoft.design/color
3. https://fluent2.microsoft.design/typography
4. https://fluent2.microsoft.design/design-tokens
5. https://fluent2.microsoft.design/shapes
6. https://fluent2.microsoft.design/layout
7. https://fluent2.microsoft.design/elevation
8. https://fluent2.microsoft.design/iconography
9. https://fluent2.microsoft.design/motion

**Microsoft Learn（Windows 设计指南）**

10. https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/materials
11. https://learn.microsoft.com/en-us/windows/apps/design/style/mica
12. https://learn.microsoft.com/en-us/windows/apps/design/style/acrylic
13. https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/layering
14. https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/geometry
15. https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/typography
16. https://learn.microsoft.com/en-us/windows/apps/design/style/color
17. https://learn.microsoft.com/en-us/windows/apps/design/style/xaml-theme-resources
18. https://learn.microsoft.com/en-us/windows/apps/design/controls/buttons
19. https://learn.microsoft.com/en-us/windows/apps/design/controls/navigationview
20. https://learn.microsoft.com/en-us/windows/apps/develop/ui/system-backdrops

**Microsoft 官方仓库源码（microsoft/microsoft-ui-xaml，main 分支 `controls/dev/CommonStyles/`）**

21. `Common_themeresources_any.xaml`（249 条 Color 定义：Light 83 / Dark 83 / HighContrast 83 + 渐变刷）
22. `Common_themeresources.xaml`（TextControl 尺寸与焦点资源）
23. `Button_themeresources.xaml`（三种按钮样式、`ControlElevationBorderBrush` 渐变）
24. `TextBox_themeresources.xaml`（`TextControlElevationBorderBrush` / `...FocusedBrush`）
25. `CheckBox_themeresources.xaml`
26. `ToggleSwitch_themeresources.xaml`
27. `Slider_themeresources.xaml`
28. `CornerRadius_themeresources.xaml`（`ControlCornerRadius=4,4,4,4` / `OverlayCornerRadius=8,8,8,8`）

**Microsoft 官方仓库源码（microsoft/fluentui，master 分支 `packages/tokens/`）**

29. `src/global/colors.ts`（grey ramp、alpha 系列、shared color 变体）
30. `src/global/brandColors.ts`（brandWeb / brandTeams / brandOffice / brandTeamsV21）
31. `src/global/colorPalette.ts`（`statusSharedColors` / `mappedStatusColors`）
32. `src/global/typographyStyles.ts`
33. `src/global/fonts.ts`（fontSize / lineHeight / fontWeight / fontFamily）
34. `src/global/spacings.ts`
35. `src/global/borderRadius.ts`
36. `src/global/strokeWidths.ts`
37. `src/global/durations.ts`
38. `src/global/curves.ts`
39. `src/alias/lightColor.ts`、`src/alias/darkColor.ts`
40. `src/alias/lightColorPalette.ts`、`src/alias/darkColorPalette.ts`（status / palette token 规则）
41. npm `@fluentui/tokens` → `lib/utils/shadows.js`（shadow2/4/8/16/28/64 公式）

**第三方（仅作参考，已在正文标为非官方）**

42. https://designmd.app/en/library/fluent-design-2 —— 用于 Acrylic 模糊半径的兜底建议（`blur(30px)` / `blur(60px)`、noise 0.02），**非官方数值**

**本次抓取失败 / 未取到权威值的条目**

- `https://learn.microsoft.com/en-us/windows/apps/design/style/layering` → 404（正确路径为 `signature-experiences/layering`）
- `TabView_themeresources.xaml` 抓取失败 → TabView 尺寸无权威值
- Button / TextBox 默认 `MinHeight = 32px` 未在开源主题资源中出现（框架默认样式提供）
- `AccentFillColorSecondary / Tertiary` 的 XAML 定义未在公开主题资源中导出
- Acrylic 的模糊半径、tint opacity 官方数值未公开
- Fluent 2 layout 页未给「内容最大宽度」的固定 px 值
