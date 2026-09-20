---
name: AudioLink
description: 局域网低延迟音频广播控制台 —— 基准语言为 Microsoft Fluent 2（Windows 11 口径），macOS 为桌面端可选外观，Android 以 Material 3 结构承载同一套令牌与状态语义。
colors:
  # ── 基准语言 Fluent 2：深色（默认，夜间听音是主场景） ──
  window: "#202020"
  chrome: "#272727"
  surface: "#2B2B2B"
  surface-2: "#323232"
  sunken: "#1A1A1A"
  line: "rgba(255,255,255,0.08)"
  line-strong: "rgba(255,255,255,0.14)"
  text: "#FFFFFF"
  text-2: "#C5C5C5"
  text-3: "#9A9A9A"
  accent: "#4CC2FF"
  accent-hover: "#6BCDFF"
  accent-pressed: "#8AD8FF"
  accent-on: "#003A5C"
  ok: "#6CCB5F"
  warn: "#FCE100"
  idle: "#9A9BA3"
  danger: "#FF99A4"
  layer: "rgba(58,58,58,0.298)"
  # ── 基准语言 Fluent 2：浅色（同一台机器的日间版本，不是深色反转） ──
  window-light: "#F3F3F3"
  chrome-light: "#F9F9F9"
  surface-light: "#FBFBFB"
  surface-2-light: "#F5F5F5"
  sunken-light: "#EDEDED"
  line-light: "rgba(0,0,0,0.06)"
  line-strong-light: "rgba(0,0,0,0.14)"
  text-light: "#1B1B1B"
  text-2-light: "#616161"
  text-3-light: "#6E6E73"
  accent-light: "#0F6CBD"
  accent-hover-light: "#1C7ECB"
  accent-on-light: "#FFFFFF"
  ok-light: "#0F7B0F"
  warn-light: "#9D5D00"
  idle-light: "#6E6E73"
  danger-light: "#C42B1C"
  layer-light: "rgba(255,255,255,0.502)"
typography:
  caption:
    fontFamily: "Segoe UI Variable Text, Segoe UI, Microsoft YaHei UI, system-ui, sans-serif"
    fontSize: "12px"
    fontWeight: 400
    lineHeight: 16
  body:
    fontFamily: "Segoe UI Variable Text, Segoe UI, Microsoft YaHei UI, system-ui, sans-serif"
    fontSize: "14px"
    fontWeight: 400
    lineHeight: 20
  subtitle:
    fontFamily: "Segoe UI Variable Text, Segoe UI, Microsoft YaHei UI, system-ui, sans-serif"
    fontSize: "20px"
    fontWeight: 600
    lineHeight: 28
  title:
    fontFamily: "Segoe UI Variable Display, Segoe UI Variable Text, Segoe UI, sans-serif"
    fontSize: "28px"
    fontWeight: 600
    lineHeight: 36
  numeral:
    fontFamily: "Cascadia Mono, Cascadia Code, Consolas, IBM Plex Mono, monospace"
    fontSize: "12px"
    fontWeight: 400
    lineHeight: 16
    letterSpacing: "0.01em"
rounded:
  small: "2px"
  control: "4px"
  card: "8px"
  overlay: "8px"
  pill: "999px"
  macos-control: "6px"
  macos-card: "10px"
  macos-panel: "12px"
spacing:
  base: "4px"
  xs: "4px"
  sm: "8px"
  md: "12px"
  lg: "16px"
  xl: "20px"
  xxl: "24px"
components:
  button-accent:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.accent-on}"
    rounded: "{rounded.control}"
    padding: "5px 11px 6px"
    height: "32px"
  button-standard:
    backgroundColor: "{colors.layer}"
    textColor: "{colors.text}"
    rounded: "{rounded.control}"
    padding: "5px 11px 6px"
    height: "32px"
  button-subtle:
    backgroundColor: "transparent"
    textColor: "{colors.text}"
    rounded: "{rounded.control}"
    padding: "5px 11px 6px"
    height: "32px"
  card:
    backgroundColor: "{colors.layer}"
    textColor: "{colors.text}"
    rounded: "{rounded.card}"
    padding: "16px"
  flyout:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.text}"
    rounded: "{rounded.overlay}"
    padding: "12px 14px"
  input:
    backgroundColor: "{colors.layer}"
    textColor: "{colors.text}"
    rounded: "{rounded.control}"
    padding: "5px 10px 6px"
    height: "32px"
  segmented:
    backgroundColor: "{colors.layer}"
    textColor: "{colors.text-2}"
    rounded: "{rounded.control}"
    height: "32px"
---

# AudioLink 设计系统（DESIGN.md）

> **本文是项目设计的唯一权威（single source of truth），机读令牌在 YAML frontmatter。**
> 上游依据（规范调研，只读）：`docs/design/fluent-2.md`（Fluent 2 / Windows 11，**生效**）、`docs/design/macos-hig.md`（macOS HIG，**已归档，不生效**）。
> 视觉基准（可运行样张）：`docs/design/preview/fluent.html`（**唯一**；macOS 样张已移入 `docs/design/archive/`）。
> 落地目标：`desktop/`（Tauri 2 + React 19 + Tailwind v4）与 `android/`（Kotlin + Compose M3）。
> 效力顺序：**本文 > 样张 > 规范调研稿 > 既有代码**。界面要变，先改本文。

## Overview

AudioLink 把任意节点的声音低延迟、可同步地推给局域网内的其他节点。界面的职责不是展示功能，而是让「在推什么、给谁、质量如何」在三秒内可读——使用者整晚不看界面，靠余光判断系统是否正常。

**视觉世界**：Microsoft Fluent 2（Windows 11 口径），**唯一**。桌面端是主场：Mica 窗口底 + 层叠表面 + 系统字体 + 4/8px 小圆角 + 双层焦点描边。Android 端以 Material 3 的**结构与交互惯例**承载**同一套令牌、形状与状态语义**。macOS HIG 调研稿留在 `docs/design/macos-hig.md` 与 `docs/design/archive/`，**不进入产品**。

**一条轴**（桌面端）：`data-theme` = `light` \| `dark`（光照）。深浅两套各自完整，**没有一套是另一套反转出来的**。

**允许的平台差异**：控件高度、触控目标（Android ≥48dp）、导航模式（桌面侧栏 / 手机单页）、窗口圆角与系统标题栏（由 OS 提供）、字体族。
**不允许的差异**：状态色的语义与文案、强调色色相、字号档位、语义色用途、错误文案口径。

## Colors

颜色分三层：**global（原始值）→ alias（语义，界面只准用这层）→ 组件**。界面里不允许出现裸十六进制。

| 语义 | 深色（默认） | 浅色 | 用途 |
|---|---|---|---|
| `window` | `#202020` | `#F3F3F3` | 窗口底（Mica 降级时的纯色兜底） |
| `chrome` | `#272727` | `#F9F9F9` | 侧栏 / 工具栏（自有底色层） |
| `surface` | `#2B2B2B` | `#FBFBFB` | 卡片 / 面板基底 |
| `surface-2` | `#323232` | `#F5F5F5` | 悬停 / 次级表面 |
| `sunken` | `#1A1A1A` | `#EDEDED` | 下沉槽（输入框、下拉、滑轨底座） |
| `layer` | `rgba(58,58,58,.298)` | `rgba(255,255,255,.502)` | 官方 LayerFillColorDefault 之上的填充层 |
| `line` / `line-strong` | `rgba(255,255,255,.08)` / `.14` | `rgba(0,0,0,.06)` / `.14` | 描边 / 强调描边 |
| `text` | `#FFFFFF` | `#1B1B1B` | 正文 |
| `text-2` | `#C5C5C5` | `#616161` | 次要文字（说明、单位） |
| `text-3` | `#9A9A9A` | `#6E6E73` | 三级文字（指纹、时间戳）；深 5.03:1 / 浅 4.90:1 |
| `accent` | `#4CC2FF` | `#0F6CBD` | 主操作、选中态、焦点强调 |
| `accent-on` | `#003A5C` | `#FFFFFF` | accent 之上的文字 |
| `ok` | `#6CCB5F` | `#0F7B0F` | 推流中 / 接收中 |
| `warn` | `#FCE100` | `#9D5D00` | 网络不稳 / 重连中 |
| `danger` | `#FF99A4` | `#C42B1C` | 断开 / 失败 |
| `idle` | `#9A9BA3` | `#6E6E73` | 空闲（中性，不抢注意力）；状态灯与 idle 文字都用它（5.12:1 / 4.90:1）。**浅色旧值 `#82838C` 只有 3.64:1，已废** |

**配对法则（血泪）**：**Fluent 的令牌编号不是配对契约**。`brandWeb[80]`（`#0F548C`）与 `brandWeb[110]`（`#62ABF5`）在原体系里分别服务「品牌底」与「中性底上的品牌文字」，把编号相邻的两个 token 当前景/背景对子用，实测只有 **3.25:1**。凡是 `on*` / `*Ink` 类令牌，取值**必须按实际配对算对比度**（正文 ≥4.5:1，图形/大字 ≥3:1）。

**同族第二案：调色板 ramp ≠ theme token**。Fluent 2 有两套并行的颜色来源 —— **brandWeb 调色板 ramp**（`brand-40/60/80/100/110`）与 **theme token**（`colorBrandBackground` 一族）。它们**不是同一个体系，编号更不是同一回事**：`brandWeb[80] = #0F6CBD`，而 dark theme 的 `colorBrandBackground = #4CC2FF`（**ramp 里根本没有这个值**）。落地时 `--al-accent` 曾错取 `brand-100 = #479EF5`，真机采样主按钮填充确为 `#479EF5`，其上 `accent-on` 文字对比度 **5.95:1 → 4.25:1**（跌破 4.5）。

**规则**：语义角色（accent 等品牌前景/底色）**只准取 theme token**，注释里写 token 名（`colorBrandBackground`）而不是档位编号；ramp 只作色阶参考。桌面端令牌命名为 `--al-global-accent-{rest,hover,pressed}[-light]`。

**状态灯底座**：滑块/进度轨道未填充部分用 Fluent `ControlStrongFillDefault` —— 深 `rgba(255,255,255,.544)`（在 `#2B2B2B` 上 5.29:1）、浅 `rgba(0,0,0,.446)`（在 `#FBFBFB` 上 3.29:1）。**不要**用 `outline`（`#616161` 在深色下仅 2.29:1，低于非文字 UI 的 3:1 门槛）。

**材质填充**（玻璃层专用，见 Elevation & Depth）：

| 语义 | 深色 | 浅色 |
|---|---|---|
| `mica-fill`（z1 窗口底） | `rgba(22,24,32,.68)` | `rgba(242,244,250,.68)` |
| `chrome-fill`（z2 侧栏/工具栏） | `rgba(15,18,26,.64)` | `rgba(255,255,255,.62)` |
| `card-fill` + `card-veil`（z3 卡片） | `rgba(58,58,58,.298)` + `rgba(52,56,66,.38)` | `rgba(255,255,255,.502)` + `rgba(255,255,255,.26)` |
| `pop-fill`（z4 浮层，最实） | `rgba(47,50,60,.84)` | `rgba(253,253,255,.85)` |
| `card-stroke` | `rgba(255,255,255,.115)` | `rgba(0,0,0,.10)` |

**纪律**：accent 只用于主操作与选中态，**小面积**（一屏实心 accent 按钮 ≤1 枚）；语义色只表达状态，不做装饰、不做分类；状态永远**颜色 + 图标/文案**双通道，禁止只靠颜色。

## Typography

| 档位 | 字号 / 行高 | 字重 | 用途 |
|---|---|---|---|
| Caption | 12 / 16 | 400 | 指纹、单位、表头、辅助说明 |
| Body | 14 / 20 | 400 | 正文、按钮、输入框、列表 |
| Subtitle | 20 / 28 | 600 | 卡片主标题、数值读数 |
| Title | 28 / 36 | 600 | 页面标题（替代原 20px page-title 的大标题档） |
| 等宽数字 | 12 / 16 | 400 | 延迟、码率、丢包、时间码 —— **必须等宽**，避免跳动 |

**字体**：Fluent 用 `Segoe UI Variable Text`（标题档可切 `Display`）；macOS 外观用 Inter（SF Pro 的开源替代）；数字一律 `Cascadia Mono` / `IBM Plex Mono`。Android 用系统字体 + 同一套字号档（12/14/20sp）。

**禁忌**：不用 italic；不用全大写（中文无此形态，英文标签也不做）；标题字重只用 600，不引入 500/700 的新档；字号不取档位之外的中间值。

## Layout

- **窗口**：默认 1100×720，最小 1024×640，最大不设（不破版上限 3840×2160）。
- **桌面骨架**：左侧栏 280px 固定 + 右侧主区（工具栏 52px + 内容区）。内容区滚动，侧栏独立滚动。
- **间距节奏**：4 的倍数，常用 4 / 8 / 12 / 16 / 20 / 24。卡片内边距 16，卡片间距 10–16，区块间距 16–24。
- **栅格**：设备卡片在内容区内单列纵向堆叠（信息密度优先于多列铺排）；窗口 ≥1600px 时可两列，间距 16。
- **控件高度**：桌面 32px（输入框、按钮、分段控件、下拉），小图标按钮 28–32px；**Android 触控目标 ≥48dp**。
- **不破版**：桌面 1024 宽起可用；Android 360–840dp；1.3× 大字体下状态卡片不裁切。

### Android 控制台（2026-09-20）

- 保持单页控制台，顶部提供「接收声音 / 发送声音」同级导航，默认接收；切换视图只改变展示，不启停服务或改变采集源。使用 Fluent 顶部导航式的中性文字、图标与短下划线选中指示，触控高度至少 48dp，指示位移 200ms，关闭系统动画时立即切换。
- 接收未连接时只展示连接表单与同一局域网的引导，不展示空遥测和禁用音量；连接后先显示真实状态、来源、系统媒体音量和断开操作，再显示设备列表。「连接另一台主机」按需展开，配对输入始终可见。
- 发送视图直接提供本机地址、等待接入、采集源和推流控制，不再把正常功能放入诊断。配对请求与错误在两个视图均可见。
- 顶栏「设置」进入二级页，集中主题、省电引导、更新、开源许可和默认折叠的诊断。系统返回逐级返回，保留表单与滚动位置。
- 内容居中，最大宽度 680dp；左右 16dp、区块间距 16dp。维持现有蓝灰令牌、无玻璃的实体表面和 4/8dp 形状。
- Android 字阶落实为 Caption 12/16、Body 14/20、Subtitle 20/28、Title 28/36；标题 600。移除旧控制台的全大写与加宽字距，读数沿用等宽数字。

#### Android Fluent 细节强化（2026-09-20，第二轮）

保留 Fluent 2，并将控件从仅映射色板推进到完整的视觉语法：

- 页面标题使用 Title 28/36，顶栏品牌使用 Body；导航不再占用大面积强调色，主色留给连接动作、选中指示和输入焦点。
- 卡片维持 8dp：0.5dp 细描边、顶部微亮 / 底部微暗内边。使用本文已有 `line`、`line-strong`、`ControlFillColorDefault` 与内边令牌，Android 不采样壁纸、不叠加实时模糊。
- 输入框使用外置标签、下沉填充、4dp 圆角和底部描边；聚焦时底边 2dp accent，键盘与文本编辑仍由 Compose 原生文本控件负责。
- 次按钮使用中性 `ControlFillColorDefault` 填充与细描边；主按钮保留 accent，按内容宽度靠右放置，触控高度 ≥48dp。禁用态保持中性。
- 主题选择以可操作的浅 / 深 / 系统界面缩略图表达，选中态带勾选与描边；缩略图仅用于设置选择，不作为主页装饰。
- 首次连接引导用「主机 → 本机」方向说明和一段准备文案，说明谁提供声音、谁主动连接。

模式依据：[Microsoft 顶部导航](https://learn.microsoft.com/en-us/windows/apps/design/basics/navigation-basics)、[Fluent Field 外置标签](https://fluent2.microsoft.design/components/web/react/core/field/usage)。具体颜色和形状仍按本文令牌；未采用已不推荐用于 Windows 11 的旧 Pivot 控件。

## Elevation & Depth

深度用**材质分层**表达，不用阴影堆叠。五层结构（z0 → z4），下层永远为上层提供可采样的背景：

| z | 层 | 配方 |
|---|---|---|
| z0 | 应用背景（用户可配单/双/三色） | `--wall-base` 纯色 + `--wall-image` 线性渐变 |
| z1 | Mica 等效（窗口底） | `mica-fill` + `backdrop-filter: blur(60px) saturate(180%)` |
| z2 | chrome（侧栏 / 工具栏） | `chrome-fill` + `blur(32px) saturate(150%)` + 1px `line` 分隔 |
| z3 | 卡片（玻璃片） | `card-fill` + `card-veil`（合计 ≈ .56 深 / .63 浅）+ `blur(16px) saturate(140%)` + 1px 描边 + 顶亮/底暗内边 + 外投影 |
| z4 | 浮层 Acrylic | `pop-fill` + `blur(40px) saturate(160%)` + 8px 圆角 + 1px 描边 + 内高光 + 外投影 |

**判据**：玻璃要「糊得住 / 透得出 / 读得清」。壁纸色到达某一层的**累计透光率**控制在 **10–15%**（实测四层分别 11.5% / 13.9% / 2.2%，z4 因承载交互而最实）。验收时把背景切成单色——整个 UI 必须被明显染色，否则就是糊成了灰板。

**阴影词汇**（只用于浮起的表面，其余用 1px 描边）：

```css
--shadow-card:    0 1px 2px rgba(0,0,0,.28);            /* 卡片（深色） */
--shadow-overlay: 0 8px 16px rgba(0,0,0,.44);           /* 浮层 */
--shadow-flyout:  0 12px 32px -10px rgba(0,0,0,.62);    /* 浮层（样张口径） */
```

**降级**（必须实现）：系统关闭透明效果 / 省电模式 / Mica 不可用时，`window` 纯色兜底；玻璃层退化为对应纯色填充；界面不得把「底是 Mica」当作布局前提。

### 四条实测红线（2026-09-18 起生效）

1. **浮层必须脱离玻璃祖先**（React 里 = 必须 `createPortal` 到 `body`）。带 `backdrop-filter` 的元素会成为后代的 **backdrop root**，把后代能采样到的背景限制在自己的盒子内；浮层伸出盒子的部分没有内容可采样 → `blur()` 完全失效，只剩半透明底。
2. **`fixed` 也救不了**：带 `backdrop-filter` 的祖先同时是 `fixed` 后代的 **containing block**。探针实测：浮层留在玻璃祖先内改 `position: fixed`，写 `left:300px` 实际渲染在 `580px`。**只改定位、不移 DOM 是死路。**
3. **portal 必须配焦点管理**：浮层挂到 `body` 末尾后，Tab 从触发按钮进浮层会绕到页面底部。打开时程序化聚焦浮层内第一个可聚焦元素，关闭时把焦点归还触发按钮；`Esc` 关闭并同步 `aria-expanded`。
4. **玻璃不许嵌套**：同一块内容被两层 `backdrop-filter` 叠加 = 模糊两次且语义混乱。Acrylic **只属于浮层**；卡片/面板用填充与描边表达层次，不再自带 backdrop。

## Shapes

| 元素 | 桌面（Fluent） | Android（`MaterialTheme.shapes` 档位） |
|---|---|---|
| 小元素（≤32px：色块、分段选中项、滑块拇指） | 2px | `extraSmall` = **2dp** |
| 控件（按钮、输入框、下拉） | 4px | `small` = **4dp** |
| 分段外框 / 次级容器 | 4px | `medium` = **4dp** |
| 卡片 / 浮层 / 对话框 | 8px | `large` = **8dp** |
| 大片容器 | — | `extraLarge` = **8dp**（不许更大） |
| 标签 / 开关轨道 / 滑块轨道 | 999px（pill 白名单仅此四类 + 头像） | 同（`CircleShape` 仅限白名单） |

**Android 落点（M3 默认值必须显式覆盖）**：`Shapes` 五档按上表钉死；且 **M3 的 `Button` / `FilledTonalButton` / `ElevatedButton` 默认 `ButtonDefaults.shape = CircleShape`（整条药丸），根本不读 `shapes` 档位** —— 每处按钮都要显式 `shape = MaterialTheme.shapes.small`；`AlertDialog` 默认 `extraLarge` = 28dp，同理覆盖。**药丸按钮是本基准的头号视觉破绽**（Fluent 只在标签/轨道/头像上允许 pill）。

**描边纪律**：Windows 用 1px 描边代替 key shadow —— 卡片 `card-stroke`，控件 `ctrl-stroke`，输入框底边深一档（`ctrl-stroke-strong`）；聚焦时底边变 2px accent。

**焦点视觉**：双层描边 —— 外 `focus-outer`、内 `focus-inner`（深色：外白内黑；浅色：外黑内白），`outline-offset: 1px`。**禁止**用颜色变化或阴影表达 focus。

## Components

组件只消费语义令牌，不写裸色值；交互态（hover / pressed / disabled / focus）必须四态齐备。

**按钮**（三型，高度 32px，圆角 4px，字号 14，`padding: 5px 11px 6px`）：

| 类型 | 默认 | hover | pressed | disabled |
|---|---|---|---|---|
| accent（主操作，一屏 ≤1 枚） | `accent` 填充 + `accent-on` 文字 | `accent-hover` | `accent-pressed` | 填充 30% 不透明 + 文字 `text-disabled` |
| standard（次操作） | `ctrl-fill` + 1px `ctrl-stroke` | `ctrl-fill-hover` | `ctrl-fill-pressed` | 文字 `text-disabled` |
| subtle（图标/行内） | 透明 + 透明描边 | `ctrl-fill-subtle-hover` | `ctrl-fill-subtle-pressed` | 文字 `text-disabled` |

**输入框 / 下拉**：`well-fill` 底 + 1px 描边（底边 `well-bottom` 深一档）+ 圆角 4px；聚焦 → 底边 2px accent + 底色转 `input-active`。原生 `<select>` 需不透明选项底色（`option-bg`）。

**开关**：40×20 轨道 + 12px 拇指，轨道 pill；开态 accent 填充 + `accent-on` 拇指。

**分段控件**：外框圆角 4px + 2px 内边距，选中项圆角 2px 且用 accent 填充；**不是 pill**。

**滑块**：轨道 pill（4px 高）+ 16px 拇指（1px 环 + 1px 外描边）；已填充部分 accent，读数等宽右对齐；被对端锁定时显示锁图标与上限刻度。

**卡片**：`card-fill` + `card-veil` 玻璃片 + 1px `card-stroke` + 8px 圆角 + 顶亮/底暗内边（`inset 0 1px 0 rgba(255,255,255,.10)` / `inset 0 -1px 0 rgba(0,0,0,.34)`）+ 外投影；标题 14/600，描述 `text-2`；状态着色只用极淡的 tint + 左侧 2px 状态细线（不整卡染色）。

**浮层（Acrylic）**：`pop-fill` + `blur(40px) saturate(160%)`，8px 圆角，1px 描边，内高光 + 外投影，`min-width: 240px`，内边距 12/14。**必须 portal 到 body**（见红线 1–3），`position: fixed` 由 JS 按触发元素 rect 定位，`resize` / `scroll`（捕获阶段）重算。

**对话框**：遮罩用 Smoke 纯色 `rgba(0,0,0,.302)`（**不用** backdrop blur），主体用浮层材质 + 圆角 8px，进出各 200ms 淡入淡出。

**状态灯与徽标**：灯 8px 圆点 + 同色 halo；标签用 999px pill + `ctrl-fill` 底 + `ctrl-stroke` 描边 + 12px 文字。呼吸动画仅用于「进行中」状态，`prefers-reduced-motion` / Android 动画缩放为 0 时必须真正停住。

**滚动条**：细轨（12px，含 4px 透明内边距），拇指 `ctrl-stroke-strong`，悬停 `text-3`；不画箭头、不做自定义轨道底色。

**空态**：下沉槽（`well`）承载一句 `text-2` 文案 + 一个主操作入口，不画插画。

## Do's and Don'ts

**Do**

- 用语义令牌写样式；颜色只在令牌表里出现一次。
- 状态用「颜色 + 图标/文案」双通道；灯与数字并列，首屏回答三问。
- 长驻内容用填充与描边分层；玻璃只给窗口底、chrome、卡片、浮层这四类。
- 数字用等宽字体并与单位同档；布局不因数字位数变化而跳动。
- 浮层一律 portal + fixed + 焦点转移；`Esc` 关闭、`aria-expanded` 同步。
- 桌面与 Android 的状态色、文案、语义保持一致（同一盏灯，两种外壳）。
- 深色与浅色同时可用，浅色不是深色的反转（各自的对比度与层次单独验收）。

**Don't**

- 不要在带 `backdrop-filter` 的祖先里渲染浮层，也不要指望改成 `fixed` 就能绕开（红线 1–2）。
- 不要让玻璃嵌套（父子同时带 `backdrop-filter`）。
- 不要把 pill 用在按钮、卡片、输入框上（白名单：标签、开关轨道、滑块轨道、头像）。
- 不要在长驻内容上使用 Acrylic；不要在对话框遮罩上用 blur。
- 不要用 500 以下字重的标题、不要 italic、不要全大写。
- 不要用语义色做装饰或分类；不要只靠颜色传达状态。
- 不要出现裸十六进制、裸 px 色值或内联样式里的颜色。
- 不要写"未知错误"；错误文案必须给成因 + 恢复路径，协议码只出现在诊断区。

## Migration & Gaps

本节记录**现状与本文的差距**，供重构分步执行（按顺序，每步可独立回滚）。

### 桌面端（`desktop/`）—— 已落地（2026-09-18）

| 项 | 落地做法（落点） |
|---|---|
| 材质 | z0–z4 五层已在 `desktop/src/index.css` 实现：`z0 .al-desktop`（用户可配背景）、`z1 .al-mica`（blur 60）、`z2 .al-chrome`（blur 32）、`z3 .al-card`（blur 16 + veil）、`z4 .al-pop`（blur 40）。各层不透明度取本文件上文「材质填充」表的最终值 |
| 令牌命名 | 两层：`--al-global-*`（原始值）→ `--al-*`（语义 alias），`@theme` 暴露 `--color-*` / `--radius-*` / `--text-*` / `--ease-fluent`，组件只写语义工具类（`bg-surface-card` / `text-text-secondary` / `border-stroke-control` / `rounded-control` / `text-body`） |
| 玻璃嵌套 | `.al-chrome .al-card` 与 `.al-flat .al-card` 的 `backdrop-filter: none`：侧栏 / 工具栏 / 抽屉内的卡片只留填充与描边。实证：`aside .al-card` 计算值 `none`，`main .al-card` 为 `blur(16px) saturate(1.4)` |
| 浮层 | `BackgroundPanel` 走 `createPortal(document.body)` + 焦点转移（打开聚焦首个可聚焦元素、关闭归还触发按钮、Esc 关闭）。实证：浮层祖先链上带 backdrop-filter 的元素**只有它自己** |
| 页面标题 | `.al-page-title` = 28 / 36 / 600（Title 档） |
| 状态灯 idle | 独立 `--al-idle`（深 #9A9BA3 / 浅 #82838C），`.al-lamp[data-on]` 四态各有 halo + glow |
| 诊断抽屉 | 保持结构，材质换成 z2（`.al-chrome.al-chrome-t`）+ 1px 描边分层；内部卡片由 `.al-chrome .al-card` 退掉 backdrop |
| 背景（新增能力） | 单色 / 双色 / 三色 + 方向；状态存 `settings.json` 的 `background` 键（与 `locale` / `autostart` 同一条路），Rust 侧 `settings.rs` 校验 `#rrggbb` |
| 设计语言 | `data-style` 那一条轴已删除（只保留 Fluent 2）；主题轴保留 `data-theme` |

### Android 端（`android/`）

| 项 | 当前实现 |
|---|---|
| 调色板 / 状态色 | `Color.kt` / `Theme.kt` 已对齐本文蓝灰令牌与 ok / warn / danger / idle，深浅模式配套 |
| 形状 | 2dp 小元素、4dp 控件 / 分段外框、8dp 卡片 / 浮层；所有选择控件显式取规范形状 |
| 字阶 | `Type.kt` 覆盖 M3 字阶，12 / 14 / 20 / 28sp；移除旧丝印大写与加宽字距 |
| 信息架构 | 接收 / 发送视图 + 设置二级页；接收按状态渐进展示，设备详情和诊断按需展开 |
| 平台适配 | 实体表面 + 描边，不使用 Mica / Acrylic；最大内容宽 680dp，触控目标 ≥48dp |

**M3 角色 → 基准令牌（实施对照，落点是 `ui/theme/Color.kt`）**

| M3 角色 | 深色 | 浅色 | 来源 |
|---|---|---|---|
| `background` | `#1A1A1A` | `#F3F3F3` | sunken / window |
| `surface` | `#202020` | `#FBFBFB` | window / surface |
| `surfaceContainerLowest` | `#1A1A1A` | `#EDEDED` | sunken |
| `surfaceContainerLow` | `#202020` | `#F9F9F9` | window / chrome |
| `surfaceContainer` | `#2B2B2B` | `#FBFBFB` | surface |
| `surfaceContainerHigh` | `#323232` | `#F5F5F5` | surface-2 |
| `surfaceContainerHighest` | `#3D3D3D` | `#EDEDED` | Fluent grey-24 |
| `surfaceVariant` | `#3D3D3D` | `#F5F5F5` | grey-24 / surface-2 |
| `onSurface` | `#FFFFFF` | `#1B1B1B` | text |
| `onSurfaceVariant` | `#C5C5C5` | `#616161` | text-2 |
| `outline` | `#616161` | `#8A8A8A` | grey-38 / grey-54。**只做描边**（卡片边、分隔线、输入框轮廓、未填充轨道）—— **不要拿它当三级文字**，三级文字另有 `TextColors.text3`（见文末） |
| `outlineVariant` | `#3D3D3D` | `#D1D1D1` | grey-24 / grey-82 |
| `primary` | `#4CC2FF` | `#0F6CBD` | accent |
| `onPrimary` | `#003A5C` | `#FFFFFF` | accent-on |
| `primaryContainer` | `#0F548C` | `#DCE9F7` | brandWeb[60] / 派生（浅色无官方值） |
| `onPrimaryContainer` | `#9CD3FF` | `#0F548C` | brandWeb 浅档 / brandWeb[80]。**深色按实际配对取 4.94:1** —— 原值 `#62ABF5` 仅 3.25:1（配对卡与省电白名单卡的正文），见「配对法则」 |
| `secondary` | `#C5C5C5` | `#616161` | text-2 |
| `onSecondary` | `#003A5C` | `#FFFFFF` | accent-on |
| `secondaryContainer` | `#323232` | `#F5F5F5` | surface-2 |
| `onSecondaryContainer` | `#FFFFFF` | `#1B1B1B` | text |
| `tertiary` | `#479EF5` | `#115EA3` | brandWeb[100] / brandWeb[70] |
| `onTertiary` | `#003A5C` | `#FFFFFF` | accent-on |
| `tertiaryContainer` | `#0C3B5E` | `#DCE9F7` | brandWeb[40] / 派生 |
| `onTertiaryContainer` | `#77B7F7` | `#0C3B5E` | brandWeb[120] / brandWeb[40] |
| `error` | `#FF99A4` | `#C42B1C` | danger |
| `onError` | `#442726` | `#FFFFFF` | critical-bg / accent-on |
| `errorContainer` | `#442726` | `#FDE7E9` | critical-bg |
| `onErrorContainer` | `#FF99A4` | `#C42B1C` | danger |
| `inverseSurface` / `inverseOnSurface` | `#FFFFFF` / `#202020` | `#202020` / `#FFFFFF` | text / window |
| `inversePrimary` | `#0F6CBD` | `#4CC2FF` | accent（另一主题） |
| `surfaceTint` | `= primary` | `= primary` | — |
| `scrim` | `#000000` | `#000000` | Smoke 之外的对话框遮罩 |

**`StatusColors`（`ui/theme/Theme.kt`）**：深色 `on #6CCB5F` / `warn #FCE100` / `live #FF99A4` / `idle #9A9BA3`；浅色 `on #0F7B0F` / `warn #9D5D00` / `live #C42B1C` / `idle #6E6E73`。`*Ink` 取同值（灯与文字同色）。
**门槛**：在 `surfaceContainer` 上深色 `on 6.98` / `warn 10.73` / `live 6.97` / `idle 5.12`，浅色 `on 5.26` / `warn 5.07` / `live 5.47` / `idle 4.90` —— 四档两端全部 ≥4.5:1。改任何一个值都要重算并在此登记。
**语义保持不变**：`live` 仍是「ON AIR / 失败断开」的红，必须与桌面 `--color-lamp-live: var(--t-danger)` 同义，不要改成绿色。

**`TextColors`（`ui/theme/Theme.kt`，2026-09-18 落地时新增）**：`text3` = 深 `#9A9A9A` / 浅 `#6E6E73`（5.03:1 / 4.90:1），接在 `KeyValueRow` 的**字段名**上（指纹 / 地址 / 状态 / 信任 / 采样率 / 声道数）。
**为什么必须独立成层**：Android 侧原本**没有** text-3 的承载者 —— 三级字段全走 `onSurfaceVariant`（那是 **text-2** 的值），而 `outline` 只做描边。上面这张表早先把 `outline` 的「来源」标成 `grey-38 / text-3`，属于**标注与值本身不一致**（它给的是描边灰，不是 text-3 的值），落地时按实际语义纠正。
**不要**把 `text3` 接到 `SilkLabel`：那个控件被 `ConsoleTopBar` 当按钮文字用（「主题」「许可」），三级灰写在可点击文字上会读成 disabled。

**`ControlColors`（`ui/theme/Theme.kt`，同批新增）**：`controlStrongFill` = Fluent `ControlStrongFillDefault`（深 `rgba(255,255,255,.544)` / 浅 `rgba(0,0,0,.446)`），专供滑块 / 进度轨道的**未填充**部分；`controlStroke` = Fluent `ControlStrokeColorDefault`。

**`AlControls.kt`（`ui/components/`，同批新增）**：M3 默认形状与默认禁用色的**唯一**覆盖点 —— `AlButton` / `AlOutlinedButton` / `AlTextButton` / `AlCard`。调用点一律写 `Al*`，不要在 13 个地方各贴一遍 `shape = MaterialTheme.shapes.small`（那种抄法的漏法在界面上表现为「这一颗还是药丸」的随机感）。

**被实际消费的角色/档位**（改造时必须覆盖；实测于 `android/.../ui/`）：`colorScheme` 的 `background` `surfaceContainer` `surfaceContainerHigh` `onSurface` `onSurfaceVariant` `primary` `primaryContainer` `onPrimaryContainer` `error` `errorContainer` `onErrorContainer` `outline` `outlineVariant`；`typography` 的 `bodySmall` `bodyMedium` `labelSmall` `labelLarge` `titleSmall` `titleMedium` `titleLarge`；`shapes.medium`；`statusColors.*` 四档。

**桌面端材质令牌：已补齐**。`desktop/src/index.css` 现在按 §10.1 分两层定义 `mica-fill` / `chrome-fill` / `card-fill` / `card-veil` / `card-stroke` / `pop-fill` / `pop-stroke`（含深浅各一套）。**仍未做的**：`src-tauri` 里没有 window-vibrancy / `DwmSetWindowAttribute` 代码 —— 窗口级 Mica（桌面壁纸）尚未接上，当前 z1 模糊的是应用自己的 z0 壁纸（这正是「背景可配」的用途）。接窗口 Mica 时 `.al-desktop` 应在 Mica 不可用或用户关透明效果时保留壁纸兜底。

### 未决（需人工确认）

1. **视觉世界是否为最终选型**：本文按 Fluent 2 基准固化。若决定保留 Android 的 On-Air Console 暖调个性，需要改写 Colors/Components 两节，并把本文改成"双语言并列"结构。
2. 壁纸饱和度（Fluent 浅色偏甜）是否收敛一档。
3. `page-title` 最终档位（Title 28 vs Subtitle 20）。
