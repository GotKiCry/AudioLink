# Apple macOS 设计语言规范（HIG + 当前材质体系）

> 面向实现的设计规范。目标读者：在 Tauri 2 + React + Tailwind v4 里复刻 macOS 观感的工程师。
> 所有数值均标注来源等级，请勿把【推断】当作官方规范使用。

## 来源等级约定

| 标记 | 含义 |
| --- | --- |
| 【官方】 | 现行 Apple HIG / Apple 开发者文档原文（2025-12 版 HIG，含 Liquid Glass 更新） |
| 【官方·API】 | AppKit / SwiftUI / Tauri API 参考文档原文 |
| 【官方·旧】 | 已下线但可考的 Apple 官方文档（Mac OS X 时代 HIG 存档） |
| 【逆向/社区】 | 第三方逆向工程或实机测量，非 Apple 规范值 |
| 【推断】 | 本规范给出的工程近似值，用于可实现的等价效果 |

---

# 1. 材质（Materials）

## 1.1 两种材质体系的关系【官方】

Apple 现行 HIG 把材质分成**两类**，这条分界线是本规范最重要的一条：

- **Liquid Glass**（macOS 26 / Tahoe 引入）：一个**独立的功能层**，浮在内容层之上。侧边栏、标签栏、工具栏、菜单、浮出层、表单（sheet）都属于这一层。
- **标准材质（Standard materials / vibrancy）**：在**内容层内部**做视觉区分（分组、层次、背景）。

原话要点：

- “Don’t use Liquid Glass in the content layer.” —— 内容层不要用玻璃。
- 例外：内容层里的**瞬态交互元素**（slider、toggle）在**被激活那一刻**会临时呈现 Liquid Glass 外观，用来强调“我正在被你操作”。
- “Use Liquid Glass effects sparingly.” —— 只在最重要的功能元素上自定义玻璃效果；系统组件自动获得该外观，不要到处手搓。
- 标准材质用于 “convey a sense of structure in the content beneath Liquid Glass”（在玻璃层之下构建层级）。

**判据（什么时候是玻璃，什么时候仍是 vibrancy）**：

| 场景 | 用哪个 | 依据 |
| --- | --- | --- |
| 窗口侧边栏、工具栏、标签栏、菜单栏下拉、右键菜单、弹出层、表单、HUD | Liquid Glass（系统组件自带） | 【官方】 |
| 应用内容背景、分组卡片、表格斑马纹、文档页面底 | 标准材质 / 纯色（contentBackground / underPageBackground） | 【官方】 |
| 内容层里的 slider / toggle 被按下时 | 临时 Liquid Glass（只在那一下） | 【官方】 |
| 媒体（照片/视频）之上的浮层控件 | Liquid Glass 的 **clear** 变体 + 必要时 35% 不透明度暗色遮罩 | 【官方】 |
| 窗口**失焦（inactive）** | 材质退场 —— 非 key 窗口不使用材质，视觉上“退后” | 【官方】 |

## 1.2 Liquid Glass 的两个变体【官方】

| 变体 | 视觉 | 用在哪 | 绝不用在哪 |
| --- | --- | --- | --- |
| **regular**（默认） | 模糊 + 调整背景亮度，保证前景文字可读 | 绝大多数系统组件；文字量大的组件：弹窗、侧边栏、浮出层 | —— |
| **clear**（高透） | 高度透明，几乎不改变背景 | 浮在**媒体内容**（照片、视频）之上的控件，追求沉浸 | 背景可能造成可读性问题的场景 |

- clear + 亮背景：加 **35% 不透明度的暗色 dimming 层**（Apple 明写 35%）。
- clear + 已经足够暗的背景（或 AVKit 自带遮罩）：不要再加 dimming。
- 两者都会随系统设置变化：用户的外观偏好、Reduce Transparency、Increase Contrast 都会改变其表现。

## 1.3 macOS 标准材质全表（NSVisualEffectView.Material）【官方·API】

14 个在用材质 + 5 个已废弃。“用在哪”是 AppKit 文档原文，“绝不用在”是本规范的推论（基于语义边界）。

| 系统名 | 中文名 | 用在哪 | 绝不用在哪 |
| --- | --- | --- | --- |
| `titlebar` | 标题栏材质 | 窗口标题栏 | 窗口主体内容区 |
| `selection` | 选中态材质 | 用于指示选中（如列表/集合中被选中的项） | 整块面板背景 |
| `menu` | 菜单材质 | 菜单（菜单栏下拉、上下文菜单） | 常驻面板 |
| `popover` | 浮出层材质 | popover 窗口背景 | 主窗口内容 |
| `sidebar` | 侧边栏材质 | 窗口侧边栏背景 | 主内容区、卡片 |
| `headerView` | 头/脚视图材质 | **行内**的 header/footer 视图（吸附在滚动内容里的表头/表尾） | 窗口级工具栏 |
| `sheet` | 表单材质 | sheet（模态工作表）窗口背景 | 普通窗口 |
| `windowBackground` | 窗口背景材质（不透明） | 不透明窗口背景 | 侧边栏（那是 sidebar 材质的地盘） |
| `hudWindow` | HUD 材质（暗） | HUD 窗口背景（浮动控制面板、overlay） | 文档窗口 |
| `fullScreenUI` | 全屏界面材质 | 全屏模态界面的背景 | 窗口化场景 |
| `toolTip` | 提示气泡材质 | tooltip 背景 | 常驻 UI |
| `contentBackground` | 内容背景材质（不透明） | 不透明内容的背景 | 侧边栏/工具栏 |
| `underWindowBackground` | 窗口底下层 | 展示在窗口背景**之下**（桌面色调渗透的底层） | 前景任何元素 |
| `underPageBackground` | 页面底下层 | 文档页面背后的区域（滚动到边界时露出的底） | 内容页本身 |

已废弃（不要新用）：`appearanceBased`、`light`、`dark`、`mediumLight`、`ultraDark`。

macOS 材质还有两个**混合模式**（NSVisualEffectView.BlendingMode）：`behindWindow`（采样窗口**背后**的桌面/其它窗口）与 `withinWindow`（采样**同窗口内**下层内容）。工程含义：侧边栏通常用 behindWindow，窗口内的浮层用 withinWindow。

## 1.4 材质的视觉配方

### 逆向得到的真实结构【逆向/社区】（Oskar Groth，2025-12；开源库 MaterialView）

层栈：

~~~
NSVisualEffectView.layer
└─ container (CALayer, masksToBounds)
   ├─ backdrop (CABackdropLayer)   ← 采样背后的内容并做滤镜
   └─ tint     (CALayer)           ← 颜色覆盖层，带 compositingFilter
~~~

关键参数：

| 参数 | 值 | 说明 |
| --- | --- | --- |
| backdrop.scale | `0.25` | 背板按 **1/4 分辨率**采样后再模糊 —— 这就是 macOS 大面积毛玻璃不卡的原因 |
| backdrop.bleedAmount | `10.0`（pt） | 采样区域向外多取 10pt，避免边缘模糊出接缝 |
| 滤镜顺序 | colorSaturate → gaussianBlur → colorBrightness | 先提饱和，再模糊，最后微调亮度 |
| 典型量级 | blurRadius **20–40**（常用 30）<br>saturate **1.8–2.2**<br>brightness **+0.02 ~ +0.03** | 逆向示例值，**不是** Apple 官方配方 |
| tint 合成 | 深色材质：暗色底 + lightenBlendMode<br>浅色材质：亮色底 + darkenBlendMode | 这解释了材质“会跟着背景变色但仍保持对比”的行为 |
| rim（描边） | 内层：白色低 alpha 高光<br>外层：黑色低 alpha 暗线 | macOS 面板那圈极细的双层边，别用 1px 实线代替 |

状态优先级（同时命中时谁赢）【逆向/社区】：
Increase Contrast > Reduce Transparency > emphasized > inactive > active（默认）。

### CSS 等效配方【推断】

用于在 WebView 里复刻（假设玻璃**下方**是自己的页面内容，而不是桌面；桌面模糊见第 10 节）：

| 材质 | 浅色 | 深色 | backdrop-filter |
| --- | --- | --- | --- |
| sidebar | rgba(246,246,246,0.72) | rgba(30,30,30,0.62) | blur(30px) saturate(180%) |
| titlebar / toolbar | rgba(246,246,246,0.62) | rgba(28,28,28,0.55) | blur(24px) saturate(180%) |
| headerView（吸顶表头） | rgba(250,250,250,0.72) | rgba(38,38,38,0.72) | blur(20px) saturate(160%) |
| menu / popover | rgba(246,246,246,0.82) | rgba(40,40,40,0.82) | blur(28px) saturate(180%) |
| hudWindow | rgba(60,60,60,0.75) | rgba(50,50,50,0.72) | blur(28px) saturate(140%) |
| contentBackground（纯色，无玻璃） | #FFFFFF | #1E1E1E | 不用 |
| windowBackground | #ECECEC | #323232 | 不用 |

再补一层 1px 内描边做 rim【推断】：
box-shadow: inset 0 0 0 0.5px rgba(255,255,255,0.35), 0 0 0 0.5px rgba(0,0,0,0.12);

---

# 2. 颜色

## 2.1 系统色（System colors）—— HIG 统一色板【官方】

数值来源：**直接对 Apple 官方色板 PNG 取像素**（HIG 颜色页把色值渲染成色块图，不提供文本 hex）。下表即 sRGB 十六进制。

| 名称 | Default (light) | Default (dark) | Increased contrast (light) | Increased contrast (dark) |
| --- | --- | --- | --- | --- |
| Red | #FF383C | #FF4245 | #E9152D | #FF6165 |
| Orange | #FF8D28 | #FF9230 | #C55300 | #FFA056 |
| Yellow | #FFCC00 | #FFD600 | #A16A00 | #FEDF43 |
| Green | #34C759 | #30D158 | #008932 | #4AD968 |
| Mint | #00C8B3 | #00DAC3 | #008575 | #54DFCB |
| Teal | #00C3D0 | #00D2E0 | #008198 | #3BDDEC |
| Cyan | #00C0E8 | #3CD3FE | #007EAE | #6DD9FF |
| Blue | #0088FF | #0091FF | #1E6EF4 | #5CB8FF |
| Indigo | #6155F5 | #6D7CFF | #564ADE | #A7AAFF |
| Purple | #CB30E0 | #DB34F2 | #B02FC2 | #EA8DFF |
| Pink | #FF2D55 | #FF375F | #E7124D | #FF8AC4 |
| Brown | #AC7F5E | #B78A66 | #956D51 | #DBA679 |

**注意：这是 2025-06-09 HIG 更新后的新值**（HIG 更新日志原文：“Updated system color values, and added guidance for Liquid Glass”）。与 2024 年之前的旧值有可见差异：

| 名称 | 旧 light | 新 light | 旧 dark | 新 dark |
| --- | --- | --- | --- | --- |
| Blue | #007AFF | #0088FF | #0A84FF | #0091FF |
| Red | #FF3B30 | #FF383C | #FF453A | #FF4245 |
| Orange | #FF9500 | #FF8D28 | #FF9F0A | #FF9230 |
| Teal | #30B0C7 | #00C3D0 | #40C8E0 | #00D2E0 |
| Indigo | #5856D6 | #6155F5 | #5E5CE6 | #6D7CFF |
| Purple | #AF52DE | #CB30E0 | #BF5AF2 | #DB34F2 |

（旧值为 2020–2024 年间广泛引用的 iOS/macOS 系统色，出自社区实测。本规范建议**统一采用新值**，否则与 Tahoe 系统 UI 并排时会明显偏色。）

## 2.2 灰度色【官方】

macOS 只有 systemGray 一个灰度语义色（#8E8E93，深浅相同）。下面的 gray 2–6 属于 **iOS/iPadOS** 体系，macOS 不提供，但 Web 侧常用来做状态色阶：

| 名称 | light | dark | a11y light | a11y dark |
| --- | --- | --- | --- | --- |
| Gray | #8E8E93 | #8E8E93 | #6C6C70 | #AEAEB2 |
| Gray 2 | #AEAEB2 | #636366 | #8E8E93 | #7C7C80 |
| Gray 3 | #C7C7CC | #48484A | #AEAEB2 | #545456 |
| Gray 4 | #D1D1D6 | #3A3A3C | #BCBCC0 | #444446 |
| Gray 5 | #E5E5EA | #2C2C2E | #D8D8DC | #363638 |
| Gray 6 | #F2F2F7 | #1C1C1E | #EBEBF0 | #242426 |

## 2.3 强调色（AccentColor / controlAccentColor）【官方 + 实测】

- macOS 的强调色由用户在「系统设置 > 外观」选择，**默认就是 system blue**。
- 语义 API：AppKit 的 controlAccentColor，SwiftUI 的 Color.accentColor。
- 可选值：blue（默认）、purple、pink、red、orange、yellow、green、graphite、multicolor（随内容变色）。
- 默认值：**light #0088FF / dark #0091FF**（= 2.1 表里的 Blue）。
- **graphite 特例**：用户选 graphite 时，macOS 会让窗口背景吸收当前桌面壁纸的色调（desktop tinting）。所以自定义组件若要参与这种“和谐感”，只在**中性态**（无彩色的控件背景/边框）加一点透明度；**彩色态不要加透明**，否则颜色会随窗口位置/壁纸变化而漂移。【官方，Dark Mode 页】
- 侧边栏图标默认使用应用强调色；用户改系统强调色时，侧边栏图标要跟着变。【官方，Sidebars 页】

## 2.4 语义色（AppKit 动态色）

### 文本与分隔线

Apple 未公开这些颜色的精确分量。下表是第三方对 Mac Catalyst / AppKit 动态色的解析（2020），**仅供实现参考**；同时给出广泛流传的 iOS 侧对应值做交叉验证：

| 语义色 | light | dark | 备注 |
| --- | --- | --- | --- |
| labelColor（主文本） | rgba(0,0,0,0.85) | rgba(255,255,255,0.85) | 【逆向/社区】记为 0.8；两版值在 0.8–0.85 之间，按 0.85 更接近现代观感 |
| secondaryLabelColor | rgba(0,0,0,0.50) | rgba(255,255,255,0.50) | iOS 对应 dark 为 0.55 |
| tertiaryLabelColor | rgba(0,0,0,0.26) | rgba(255,255,255,0.25) | 部分来源记 0.20 |
| quaternaryLabelColor | rgba(0,0,0,0.10) | rgba(255,255,255,0.10) | —— |
| placeholderTextColor | rgba(0,0,0,0.25) | rgba(255,255,255,0.25) | —— |
| disabledControlTextColor | rgba(0,0,0,0.20) | rgba(255,255,255,0.20) | —— |
| separatorColor | rgba(0,0,0,0.10) | rgba(255,255,255,0.10) | 1pt，低对比 |
| gridColor | #CCCCCC | rgba(255,255,255,0.10) | 表格网格线 |
| linkColor | #0068DA | #419CFF | —— |

### 容器与选中态

| 语义色 | light | dark | 用途 |
| --- | --- | --- | --- |
| windowBackgroundColor | #ECECEC | #323232 | 窗口背景 |
| underPageBackgroundColor | ——（浅色值未取得） | #282828 | 文档页面之后 |
| controlBackgroundColor | #FFFFFF | #1E1E1E | 大块控件背景（表格、浏览器视图） |
| textBackgroundColor | #FFFFFF | #1E1E1E | 文本域背景 |
| selectedContentBackgroundColor | **#0063E1** | **#0058D0** | **key 窗口**中的选中行背景 |
| unemphasizedSelectedContentBackgroundColor | —— | #464646 | **非 key 窗口**中的选中行背景（灰） |
| selectedTextBackgroundColor | #B3D7FF | #3F638B | 文本选中高亮 |
| keyboardFocusIndicatorColor | #007AFF | #007AFF | 键盘焦点环（旧值，实际随强调色） |
| findHighlightColor | #007AFF | #007AFF | 查找高亮 |
| shadowColor | #000000 | #000000 | 阴影 |

> selectedContentBackgroundColor 的实测值 #0063E1 对应的是**默认蓝色强调色下的“系统选中蓝”**（比 systemBlue 更沉）。这一条在复刻侧边栏选中态时非常关键：选中块不是 #0088FF。【逆向/社区】

## 2.5 深色模式规则【官方】

- 不要提供应用级的外观开关 —— 跟随系统。
- 对比度下限：**正文 ≥ 4.5:1**，自定义前景/背景尽量 **7:1**（小字号尤其）。
- 深色不是浅色的简单反相：背景更暗、前景更亮，但并非逐色翻转。
- 用语义色（labelColor / controlColor / separator），不要硬编码 hex。
- 深色下白底图片要压低亮度，避免“发光”。
- macOS 的两个特有表现：**非 key 窗口不使用材质**；**graphite 强调色触发 desktop tinting**。

---

# 3. 排版（Typography）

## 3.1 macOS 内置文本样式全表【官方】

（现行 HIG “macOS built-in text styles” 原始表，单位 pt）

| 样式 | 字重 | 字号 | 行高 | 强调态字重 | 典型用途（本规范补充） |
| --- | --- | --- | --- | --- | --- |
| Large Title | Regular | **26** | 32 | Bold | 空状态、欢迎页大标题 |
| Title 1 | Regular | **22** | 26 | Bold | 页面主标题 |
| Title 2 | Regular | **17** | 22 | Bold | 主要分区标题 |
| Title 3 | Regular | **15** | 20 | Semibold | 次级分区标题 |
| Headline | **Bold** | **13** | 16 | Heavy | 与正文同字号但加粗的行首强调（列表项标题） |
| Body | Regular | **13** | 16 | Semibold | 正文（macOS 默认字号） |
| Callout | Regular | **12** | 15 | Semibold | 说明文字、次要信息 |
| Subheadline | Regular | **11** | 14 | Semibold | 小节说明、副标题 |
| Footnote | Regular | **10** | 13 | Semibold | 脚注、时间戳 |
| Caption 1 | Regular | **10** | 13 | Medium | 角标、最小注释 |
| Caption 2 | **Medium** | **10** | 13 | Semibold | 角标（更弱） |

配套事实【官方】：

- macOS **默认字号 13 pt，最小 10 pt**（平台规格表）。
- macOS **不支持 Dynamic Type**（HIG 原文：“macOS doesn’t support Dynamic Type”），不要做跟随系统字号的缩放逻辑。
- 强调态是**换字重**，不是换字号：Headline 的 emphasized 是 Heavy，Caption 1 是 Medium。

## 3.2 字距（Tracking）【官方】

macOS 字距表（单位 pt；原表覆盖 6–96 pt，这里取常用档）：

| 字号 (pt) | Tracking (1/1000 em) | Tracking (pt) |
| --- | --- | --- |
| 10 | +12 | +0.12 |
| 11 | +6 | +0.06 |
| 12 | 0 | 0.00 |
| **13** | **−6** | **−0.08** |
| 14 | −11 | −0.15 |
| 15 | −16 | −0.23 |
| 16 | −20 | −0.31 |
| 17 | −26 | −0.43 |
| 18 | −25 | −0.44 |
| 20 | −23 | −0.45 |
| 22 | −12 | −0.26 |
| 24 | +3 | +0.07 |
| 26 | +8 | +0.22 |

工程含义：**小字号紧排（负字距），大字号松排**。13 pt 正文对应 letter-spacing: -0.08px（约 -0.006em）。曲线在 12 pt 过零、17–20 pt 最紧、24 pt 之后转正。

## 3.3 SF Pro 的 optical size【官方】

- macOS 系统字体是 **SF Pro**（San Francisco 家族：SF Pro / SF Compact / SF Arabic / SF Armenian / SF Georgian / SF Hebrew / SF Mono，以及圆角变体）。另有衬线族 **New York**（macOS 上仅 Mac Catalyst 可用）。
- 系统以**可变字体（variable font）**提供，支持**动态光学尺寸（dynamic optical sizing）**：过去分立的 Text / Display 光学尺寸已合并为一条连续曲线，系统按 pt 大小对每个字形插值 —— 工程上**不需要**自己切换 Text/Display 版本，只要指定字号。
- 字重范围 Ultralight → Black，另有 Condensed / Expanded 宽度。
- HIG 明确建议**避免 light 以下字重**（Ultralight / Thin / Light），小字尤甚；优先 Regular / Medium / Semibold / Bold。
- 若要在设计稿里精确还原，可能需要手动调 tracking（运行时系统会自动调）。

## 3.4 中文字体回退【推断】

HIG 不涉及中文回退，以下是 macOS 平台事实 + 工程建议：

- SF Pro **不含 CJK 字形**，macOS 会回退到 **PingFang SC（苹方-简）**；繁体走 PingFang TC，日文走 Hiragino。
- Web 侧字体栈应显式写出，避免落到 serif 或 Windows 字体：
  -apple-system, BlinkMacSystemFont, “SF Pro Text”, “SF Pro Display”, “PingFang SC”, “Hiragino Sans GB”, “Microsoft YaHei”, system-ui, sans-serif
- 中西混排：中文字面比同字号西文视觉更大更方，排版中常用 +1px 行高补偿；正文 13px / 行高 18–20px 观感最接近原生。
- SF Symbols 与 SF 字重一一对应，图标与相邻文本**保持同字重**（见 §5.2）。

---

# 4. 形状（Shape）

## 4.1 窗口圆角

| 版本 | 窗口圆角 | 来源 |
| --- | --- | --- |
| macOS 11 Big Sur – 15 Sequoia | **10 pt** | 【逆向/社区】（通过私有 defaults 键 NSConvolutionOverride1 实测默认值 10） |
| macOS 26 Tahoe（含工具栏） | **26 pt** | 【逆向/社区】（同上，社区实测默认 26；Apple 未公开数值） |
| macOS 26 Tahoe（仅标题栏、无工具栏） | **更小**（小于 26 pt） | 【官方】WWDC25 “Build an AppKit app with the new design”原话：Titlebar-only windows retain a smaller corner radius |
| 下一代（社区称 Golden Gate） | 20 pt | 【逆向/社区】（同一 defaults 键，仅供参考） |

Apple 官方对圆角的表述（WWDC 2025 session 310 原文）：

- “windows now have a softer, more generous corner radius, **which varies based on the style of window**”；
- “Windows with toolbars now use a larger radius, which is designed to **wrap concentrically around the glass toolbar elements**, scaling to match the size of the toolbar”；
- “Titlebar-only windows retain a smaller corner radius, wrapping compactly around the window controls”；
- 官方同时承认副作用：更大的圆角会**裁切**靠近窗口边缘的内容，并提供了 NSView.LayoutRegion API 用于把内容嵌进圆角。

**结论**：Tahoe 的窗口圆角不是常量，而是“随窗口顶部元素高度自适应”的变量。Web 侧无法复刻这种自适应，建议用**固定 20px**（窗口含工具栏时）作为近似值【推断】。

## 4.2 同心圆角（Concentricity）原则【官方】

这是 macOS 新设计体系的核心几何规则：

> 每个元素的曲率都应当“坐进”其容器的圆角里，且这种关系是**双向**的。

工程化公式【推断】：

~~~
childRadius = max(0, parentRadius - padding)
~~~

- 工具栏内标准组件的圆角默认与栏的圆角同心（HIG 原话）。
- 自绘组件必须自己保证同心，否则会明显“出戏”。
- 官方 API：SwiftUI 的 ConcentricRectangle / rect(corners:)，UIKit 的 UICornerConfiguration，AppKit 的 corner configuration。

## 4.3 控件与容器圆角【推断 / 逆向】

Apple 不公开这些数值。以下是复刻 macOS 观感的工程取值：

| 元素 | 圆角 | 备注 |
| --- | --- | --- |
| Push button（regular，22pt 高） | **5–6 px** | Big Sur–Sequoia 观感；Tahoe 更圆，可到 8–10 px |
| 文本域 / 搜索框 / pop-up button | 5–6 px（与按钮一致） | 完全同心于按钮 |
| 分段控件 | 外框 6–8 px；Tahoe 后接近**胶囊** | 段内角 = 外角 − 1px |
| 侧边栏选中块 | 5–6 px，左右内缩 6–10 px | 见 §6.6 |
| 卡片 / 分组容器 / 表单分组 | **10–12 px** | Tahoe 明确增大 section 圆角 |
| 浮出层（popover） | 10–12 px | —— |
| Sheet（表单窗口） | **明显增大**（Tahoe） | 官方原话：sheets feature an increased corner radius |
| 窗口 | 10 px（旧） / 26 px（Tahoe） | 见 §4.1 |
| 提示气泡（tooltip） | 4–6 px | —— |

## 4.4 胶囊（Capsule）用在哪【官方 + 推断】

| 控件 | 是否胶囊 | 依据 |
| --- | --- | --- |
| Switch（开关） | **是**，固定胶囊 | 【官方】toggle 的 switch 样式 |
| 侧边栏/工具栏中的玻璃按钮组 | 是（组容器胶囊、成员同心） | 【官方】Liquid Glass 控件形态 |
| 分段控件 | 旧版圆角矩形；Tahoe 后趋向胶囊 | 【推断】 |
| 主操作按钮（Done / Submit） | 否，圆角矩形 | 【推断】 |
| visionOS 按钮 | 是（图标为圆、纯文字为胶囊） | 【官方】 |
| watchOS 内联按钮 | 是 | 【官方】 |

---

# 5. 间距与布局（Layout）

## 5.1 官方数值（可直接引用）

| 项目 | 数值 | 来源 |
| --- | --- | --- |
| 菜单栏高度 | **24 pt** | 【官方】The menu bar：“The menu bar’s height is 24 pt” |
| 拆分视图分隔条（thin divider） | **1 pt** | 【官方】Split views：“The thin divider measures one point in width” |
| 按钮最小命中区 | **44×44 pt**（visionOS 为 60×60） | 【官方】Buttons |
| 弹窗内两个按钮的中心间距（visionOS 参照） | ≥ 60 pt | 【官方】 |
| 窗口标题长度 | **≤ 15 字符** | 【官方】Toolbars：“keep the title under 15 characters long” |
| 工具栏分组数 | **≤ 3 组** | 【官方】Toolbars：“aim for a maximum of three” |
| 分段控件段数 | 宽界面 **5–7 段**，iPhone 约 5 段 | 【官方】Segmented controls |
| 单选按钮组 | 2–5 个，超过约 5 个改用 pop-up button | 【官方】Toggles |

## 5.2 控件与图标尺寸

| 项目 | 数值 | 来源 |
| --- | --- | --- |
| 控件高度 regular | **22 pt（px）** | 【官方·旧】Apple HIG（Controls）：“Regular size: 22 pixels high” |
| 控件高度 small | **19 pt** | 【官方·旧】同上 |
| 控件高度 mini | **15 pt** | 【官方·旧】同上 |
| 文本框 / 搜索框高度 | 同上三档 | 【官方·旧】 |
| 新增 extra-large 控件尺寸档 | Tahoe | 【官方】Adopting Liquid Glass：“Controls also feature an option for an extra-large size” |

> 注：Sonoma 起 NSControl 的默认字号有调整（第三方实测），因此“22 pt”应视为**基准值**而非硬约束；Tahoe 上实测控件更高。

**SF Symbols 尺寸与字重对应**【官方】：

- 9 个字重（Ultralight → Black）与 San Francisco 字体的 9 个字重**一一对应**，图标与相邻文本可以做到精确的字重匹配。
- 3 个尺寸档：**small / medium（默认） / large**，定义是**相对于 SF 字体的 cap height（大写字母高度）**，而非绝对像素。
- 用法：图标与相邻文本**同字重**（“In general, match the weights of interface icons and adjacent text”）；用 scale 调整视觉分量而不破坏字重匹配。
- 渲染模式：monochrome / hierarchical / palette / multicolor；表达状态、层次时优先 hierarchical。
- 工程近似【推断】：13pt 正文搭配图标常用 **16px**（medium）；工具栏图标常用 **16–20px**；列表行图标 16px。

## 5.3 侧边栏与工具栏尺寸

Apple 现行 HIG **只给行为不给数值**：macOS 侧边栏的行高、文字、字形大小随其整体尺寸（small / medium / large）变化，用户可在「系统设置 > 通用 > 边栏图标大小」中修改。

工程取值【逆向/社区 + 推断】：

| 项目 | 取值 | 说明 |
| --- | --- | --- |
| 侧边栏宽度 | **220–260 px**（默认展开）；最小 200 px，可拖至约 300 px | 系统应用（Mail / Notes / Finder）实测区间 |
| 侧边栏最小宽度建议 | 225 pt | 【逆向/社区】（225–275 区间） |
| 侧边栏最大宽度建议 | 350–400 pt | 【逆向/社区】 |
| 侧边栏行高（medium） | **28 px**；小号 24 px | 随尺寸档变化 |
| 侧边栏图标大小 | 16 / 18 / 20 px（对应 small / medium / large） | 【推断】 |
| 工具栏（与标题栏合并）高度 | **52 px**（macOS 11+）；Tahoe 约 56 px | 【逆向/社区 + 推断】 |
| 仅标题栏高度 | 28 px | 【逆向/社区】 |
| 表格行高（macOS 标准） | **28 px**；Tahoe 增大 | 【官方】Adopting Liquid Glass：lists/tables/forms have a larger row height and padding |
| 窗口内容边距 | **20 pt** | 【推断】（Apple 旧 HIG 的窗口内容留白惯例） |

## 5.4 布局禁忌【官方】

- **不要把关键信息或操作放在窗口底部**（用户会把窗口下沿拖出屏幕外）；侧边栏底部同样禁止。
- 不要把内容放到窗口顶部摄像头刘海区域之下。
- 内容区不要被侧边栏/工具栏遮挡 —— 用安全区 + scroll edge effect 处理滚动内容与栏的关系，而不是给栏加一层实心背景色。

---

# 6. 控件规格（macOS）

## 6.1 Push button（按钮）【官方】

| 类型 | 外观 | 行为 |
| --- | --- | --- |
| **Default / Primary** | 强调色**实心填充** + 白字 | 响应 Return/Enter；在 sheet/弹窗内按 Return 会连带关闭视图 |
| **普通按钮** | 浅灰填充（controlColor）+ 极细边框，**无强调色** | —— |
| **Cancel** | 同普通按钮 | 响应 Esc |
| **Destructive（危险）** | **红色文字/图标，不填红底** | role = destructive |

官方硬规则：

- **一个视图里只放 1–2 个 prominent 按钮**（原话：“Keep the number of prominent buttons to one or two per view”）。
- 用**样式**（不是尺寸）区分主次；同尺寸并列 = 一组同级选择。
- **永远不要**把 primary 角色给破坏性操作（用户会不看就按回车）。
- 按钮点击区最小 44×44 pt；必须要有按下（press）状态。
- 标题以动词开头；打开新窗口/视图/应用的按钮，标题末尾加**省略号**（…）。
- macOS 专属类型：flexible-height push button（两行文字/高图标的场景，圆角与内边距和普通按钮一致）、square button（渐变方块按钮，只能放视图里，**不能**放工具栏/状态栏）、help button（圆形问号，每窗口最多一个）、image button（图文按钮，图片与按钮边缘之间留约 10px；一般不加系统边框）。

## 6.2 Pop-up button（弹出按钮）【官方】

- 用途：一组**互斥**选项/状态，扁平列表。
- 需要多选、需要子菜单、或列出的是**动作**时 → 改用 pull-down button。
- 必须有合理的默认选中项。
- 可以加一个 Custom… 项承载低频需求。
- 形态：左侧当前值文本 + 右侧上下双箭头指示符；高度与 push button 一致（22 pt），圆角同心。

## 6.3 Segmented control（分段控件）【官方 + 推断】

- 定义：线性排列的 2 个以上分段，每个分段是一个按钮。
- **等宽**是常态；图标与文字宽度也尽量一致。
- **单选或多选**：macOS 两种都允许（Keynote 的对齐控件是单选，字体属性控件可多选）。
- 第三种模式：**momentary（瞬时按钮组）**，不显示选中态（如 Mail 的回复/全部回复/转发）。
- **一致性铁律**：同一个控件里不要混合“动作型分段”和“选择型分段”。
- 段数：宽界面 5–7 段上限；内容尺寸尽量一致；不要混用文字与图标。
- **选中态外观**【推断，基于 macOS 实测观感】：
  - 选中段 = **实心填充**（强调色填充 + 白字；非强调场景为浅灰填充 + 主文本色）；
  - 未选中段 = 透明（露出控件底部作为轨道）；
  - 轨道 = 接近 controlBackgroundColor 的低对比底色，整体一个圆角矩形；Tahoe 后轨道与选中块圆角均增大、趋向胶囊；
  - 分段之间 1px 内分隔或无缝，靠填充块区分。
- macOS 特有：支持 **spring loading**（拖拽内容悬停在分段上，force click 即可激活）。

## 6.4 Switch / Checkbox / Radio【官方】

| 控件 | 形状 | 状态外观 | 用在哪 |
| --- | --- | --- | --- |
| **Switch** | 胶囊轨道 + 圆形滑块 | 开 = 强调色填充（**注意：macOS 用系统/应用强调色，不是 iOS 的绿色**） | 窗口**主体内**；有视觉分量的设置；成组开关 |
| **mini switch** | 同形状、小号 | 同上 | 分组表单里单行设置的开关（高度与按钮一致，保证行高统一） |
| **Checkbox** | 小方块 | 关 = 空；开 = 强调色填充 + 白色勾；**mixed = 横杠** | 需要表达层级/依赖关系的设置项；纵向对齐 + 缩进表达从属 |
| **Radio button** | 小圆 | 选中 = 实心圆；未选 = 空心 | 2–5 个互斥选项；横向排列时按最长标签统一间距 |

- Switch、Checkbox、Radio **只能放在窗口主体，不能放工具栏/状态栏**。
- 不要用 switch 替换已经存在的 checkbox（官方原话：“In general, don’t replace a checkbox with a switch”）。
- 不要只靠颜色区分开关状态。

## 6.5 Slider（滑块）【官方】

- 方向约定：横向 = 左小右大；纵向 = 下小上大。不要反。
- **macOS 两种形态**：
  - **线性滑块**：thumb 是**窄菱形（narrow lozenge）**，不是圆形；最小值到 thumb 之间的轨道用强调色填充；可带**刻度线（tick marks）**。
  - **圆形滑块**：thumb 是**小圆点**；刻度线是圆周上的均匀点。用于首尾相接/无界值（角度、旋转次数）。
- 建议配套文本框 + stepper 显示/输入精确值。
- 滑块改变时要**实时反馈**（如 Dock 图标随大小滑块实时缩放）。
- 标签以冒号结尾；刻度可只在最小/最大值处标注；悬停 thumb 显示数值提示。
- Tahoe 的变化：拖动时 thumb 会**变成 Liquid Glass**（官方原话：knob transforms into Liquid Glass during interaction）。

## 6.6 Sidebar list（侧边栏列表）【官方 + 推断】

官方要求：

- 一般**不超过两级层级**；更深的数据用 split view 加中间列表。
- 需要两级时，用简短的分组标题。
- 侧边栏可以（也应该）在内容之下“浮动”—— 让内容从下面穿过去（background extension effect / 横向滚动）。
- macOS 上侧边栏的行高、文本、字形大小随整体尺寸档变化（small / medium / large）。
- 用户可以自定义内容与顺序；优先使用 SF Symbols 作图标。
- 允许隐藏侧边栏，但要提供显示/隐藏按钮或 View 菜单命令；**不要默认隐藏**。

**选中态外观**【推断，macOS 实测观感，最关键的识别特征之一】：

- 选中行 = **内缩的水平圆角实心块**（左右各内缩 6–10 px），不是整行铺满，也不是左侧一条竖线。
- 填充色 = **强调色**（默认蓝，注意参考 §2.4 的 #0063E1 而非 #0088FF）；文字与图标变白。
- 窗口**失焦**时，选中块变**灰**（unemphasizedSelectedContentBackgroundColor，dark 约 #464646）。
- 圆角约 5–6 px（Tahoe 后增大），与侧边栏容器圆角同心。
- 悬停：浅灰半透明高亮，**不改变**文字颜色。

## 6.7 Toolbar（工具栏）【官方】

- macOS 工具栏位于窗口顶部框架内，**与标题栏合并或在其下方**；窗口标题可与控件同行显示。
- **工具栏条目默认没有边框（bezel）** —— 官方原文：“toolbar items don’t include a bezel”；并明确要求用**无边框的系统符号**：“Borders (like outlined circle symbols) aren’t necessary because the section provides a visible container”。
- 悬停/选中态由系统自动给（淡色底、可能的玻璃胶囊），**不要**自己画圆圈边框。
- **图标何时带边框**：基本不需要。唯一例外是 Tahoe 的 Liquid Glass **按钮组**（多个操作共享一个玻璃底），此时是“组容器有底”，不是“每个图标有边框”。
- 位置分区：
  - leading：返回、显示/隐藏侧边栏、标题、document menu，**不可自定义**；
  - center：常用控件，可自定义，窗口变窄时自动收进系统 overflow 菜单；
  - trailing：重要常驻项、inspector 按钮、可选的搜索框、More 菜单、主操作，**任何窗口宽度下都可见**。
- **.prominent 样式**只给一个主操作（Done / Submit），放在 trailing。
- 不要把“带文字标签的动作”和“纯图标动作”挨着放（会被看成一个复合控件）；多个文字按钮之间插入固定间距。
- 同一动作不要出现在两个分组里；分组 ≤ 3 组。
- **每一个工具栏动作都必须在菜单栏里有对应命令**（官方硬规则）。

## 6.8 菜单与菜单栏【官方】

- 菜单栏高度 **24 pt**；应用名**加粗**；顺序：Apple 菜单 → 应用名 → File → Edit → Format → View → 应用自定义 → Window → Help。
- View 菜单必须存在（哪怕只有 Enter Full Screen）；Window 菜单必须存在（哪怕只有一个窗口，因为 Full Keyboard Access 需要）。
- 不可用的菜单项**置灰而不是隐藏**（保持可发现性）。
- 菜单宽度自动按最宽项计算（含动态菜单项）。
- Tahoe 变化：菜单采用 Liquid Glass；常用动作的菜单项**带图标**（系统按 selector 自动给）；上下文菜单顶部的动作要与 swipe actions 一致。
- 菜单栏 extras 用黑白描边符号；点击应弹出**菜单**而不是 popover。

## 6.9 列表与表格【官方】

- macOS 定义 **bordered 风格**：用交替行背景（alternating row colors）帮助宽表格横向对齐；分组用头部/尾部 + 额外留白。
- 多列宽表格建议交替行色；单列列表不需要。
- 允许点击列头排序（再点一次反向）、允许拖动列宽。
- 层级数据用 **outline view**（带展开三角），不要用普通表格硬撑。
- 选中反馈：导航型表格**持续高亮**选中行；选项型表格可短暂高亮后换成勾选标记。

---

# 7. 动效（Motion）

## 7.1 Apple 官方给出的原则（无具体毫秒值）

HIG **通篇不给时长与曲线数值**，只给原则【官方】：

- 动效要“简短而精确”（brief and precise），倾向于“轻量、不打扰”。
- 大部分 UI 交互**不要**加自定义动效 —— 系统已经为标准控件提供了微妙的动画。
- 动效不能是传递关键信息的唯一方式（要配 haptic / audio / 文本）。
- 要让用户**可以打断/取消**动效，不要让人等动画放完。
- 反馈动效要符合手势直觉（从上滑入的东西不应从侧边消失）。
- 游戏帧率目标 30–60 fps。

## 7.2 Liquid Glass 的形变行为【官方】

- 交互时控件会**形变**：slider 与 toggle 的 knob **变成 Liquid Glass**；**按钮会流动地形变成菜单和浮出层**（“buttons fluidly morph into menus and popovers”）。
- 材质的运动**随输入方式分级**：直接触摸的强调更强，用触控板（trackpad）时更**克制** —— 这是官方明文，桌面端复刻时动作幅度应更小。
- 多个玻璃元素相邻时用 GlassEffectContainer 合并以优化渲染性能（官方 API 提示）。
- 形变与透明度会响应无障碍设置（Reduce Motion / Reduce Transparency）：系统组件自动适配，自定义元素必须自己测。

## 7.3 工程化时长与曲线【推断】

Apple 未公开，以下是复刻 macOS 观感的常用取值（也是 Web 上最接近原生手感的组合）：

| 场景 | 时长 | 曲线 |
| --- | --- | --- |
| 悬停高亮（hover in/out） | 100–150 ms | ease-out |
| 按下反馈（press） | 50–100 ms | ease-out |
| 开关 / 复选框状态切换 | 150–200 ms | cubic-bezier(0.25, 0.1, 0.25, 1) |
| 弹出层 / 菜单出现 | 180–220 ms | cubic-bezier(0.2, 0, 0, 1) + 轻微缩放（0.96 → 1）与淡入 |
| 侧边栏折叠/展开 | 250–300 ms | cubic-bezier(0.32, 0.72, 0, 1) |
| 窗口/面板尺寸变化 | 250–350 ms | 同上 |
| 玻璃形变（按钮 → 菜单） | 300–400 ms | cubic-bezier(0.34, 1.2, 0.64, 1)（轻微回弹） |

## 7.4 无障碍降级【官方】

| 系统设置 | 表现 | 我们要做的 |
| --- | --- | --- |
| **Reduce Motion** | 系统移除或改造部分效果（标准组件自动） | 自定义动效改为**纯淡入淡出**（0.1–0.15 s），取消位移/缩放/回弹；CSS 用 @media (prefers-reduced-motion: reduce) |
| **Reduce Transparency** | 材质变为**不透明填充**，停止背景取样 | 提供不透明回退色（sidebar 变 #F2F2F2 / #1E1E1E）；CSS 用 @media (prefers-reduced-transparency: reduce) |
| **Increase Contrast** | 材质更不透明、边框更明显、对比度提升 | 切换到 §2.1 表的 “Increased contrast” 列；分隔线 alpha 提到 0.25+ |

优先级：Increase Contrast > Reduce Transparency > emphasized > inactive > active。

---

# 8. 识别特征清单：什么让界面一眼看出是 macOS

按“可执行”标准列出，每条都能落到代码上。

| # | 特征 | 具体做法 |
| --- | --- | --- |
| 1 | **顶部全局菜单栏 24pt**，应用名加粗 | 高度 24px；左侧 Apple 菜单，右侧菜单栏 extras；用 Tauri 的原生菜单实现，不要自绘 |
| 2 | **左上角交通灯**（关闭/最小化/缩放） | 12px 圆点、间距 8px、距左 20px；**key 窗口彩色、非 key 窗口全灰** |
| 3 | **非活动窗口不用材质 + 整体变暗** | blur/材质仅在 focus 时启用；失焦时背景降饱和、阴影变浅、控件变灰 |
| 4 | **侧边栏 = 半透明材质 + 内缩圆角实心选中块** | 见 §6.6：选中块内缩 + 圆角 + 强调色填充，不是竖条也不是整行铺满 |
| 5 | **工具栏图标无边框，悬停才有容器** | 默认透明，hover 时淡色圆角底；禁止给图标画圆环边框 |
| 6 | **玻璃只在控制/导航层，内容层绝无毛玻璃** | 内容区用纯色/标准材质；只有 sidebar / toolbar / menu / popover / sheet 是玻璃 |
| 7 | **一屏只有 1 个实心填充按钮** | 其余全是灰底描边按钮；危险操作用红字而非红底 |
| 8 | **13pt 正文、紧凑的字号梯度** | Body 13 / Title3 15 / Title2 17 / Title1 22，行高比 iOS 紧得多 |
| 9 | **同心圆角** | 所有嵌套元素圆角 = 父圆角 − 内边距，形成“套娃”感 |
| 10 | **1pt 低对比分隔线**（rgba(0,0,0,0.1)） | 不用 2px、不用重色；表格用交替行色代替竖线网格 |
| 11 | **控件高 22px，紧凑不臃肿** | 按钮/输入框/下拉统一 22px（mini 15 / small 19） |
| 12 | **克制的动效：以淡入淡出为主** | 悬停 100–150ms；折叠 250–300ms；无花式弹跳、无长时加载动画 |
| 13 | **可拖拽分隔条 + 可隐藏面板** | 侧边栏/检查器都要有 1pt 分隔条可拖，并在 View 菜单提供 Show/Hide 命令 |
| 14 | **完整的菜单栏命令体系** | 工具栏里每个动作在菜单栏都有对应项与快捷键（“桌面应用”而非“网页”的关键体感） |

---

# 9. 反模式：macOS 不会做什么

| 反模式 | 为什么不对 | 正确做法 |
| --- | --- | --- |
| **整片内容区做毛玻璃** | 官方明文禁止在内容层用 Liquid Glass；背景取样会让文字可读性崩坏 | 内容层用不透明背景/标准材质；玻璃只留给控制与导航层 |
| **彩色填充按钮到处都是** | macOS 一屏通常只有 1 个 prominent 按钮 | 只给主操作强调色填充，其余灰底描边 |
| **危险操作做成红色实心按钮** | 视觉权重会诱导误点，且必须与 primary 角色分离 | 红色**文字**即可，永不与 primary 合并 |
| **直角方框 / 大圆角卡片满屏** | 破坏同心圆角体系 | 用“父圆角 − padding”推导子圆角 |
| **粗边框、重阴影、明显渐变** | macOS 的层次靠材质与 1px 细线，而非描边 | 边界用 rgba(0,0,0,0.1) 的 1px 线 + 极淡扩散阴影 |
| **忽略窗口 key / inactive 状态** | 这是 macOS 最显著的窗口语义 | 失焦时：材质退场、选中块变灰、交通灯变灰 |
| **顶部用 iOS 风格的 tab bar** | macOS 用 sidebar / segmented control / tab view | 用侧边栏或窗口内的 tab view |
| **整屏 iOS 级字号（17pt 正文）** | macOS 正文 13pt，17pt 会让应用像放大的 iPad 应用 | 严格按 §3.1 表 |
| **绿色开关（iOS 风格）** | macOS 开关使用系统/应用强调色 | 蓝色（默认强调色）或应用强调色 |
| **把关键操作放窗口/侧边栏底部** | 用户经常把窗口底边拖到屏幕外 | 关键操作放顶部工具栏或行内 |
| **只有工具栏入口、没有菜单栏命令** | 用户可隐藏工具栏；Full Keyboard Access 用户依赖菜单 | 每个工具栏动作都在菜单栏提供命令 + 快捷键 |
| **深色模式用纯黑 + 纯白** | 对比过强、发光感重 | 用 #1E1E1E / #323232 级别的暗底 + 0.85 alpha 文本 |
| **自绘窗口边框/交通灯** | 官方明确反对：无法完美匹配系统行为会让应用显得是坏的 | 用原生窗口装饰 |
| **给高频交互加长动画** | 官方明确不建议 | 高频交互的反馈 < 150ms，甚至不刻意加 |

---

# 10. 给 Tauri / React 的落地映射

## 10.1 关键约束：CSS 毛玻璃 ≠ macOS 材质

| 能力 | 能做到什么 | 做不到什么 |
| --- | --- | --- |
| backdrop-filter | 模糊**页面内**位于该元素下方的 DOM 内容 | **无法**模糊窗口背后的桌面/其它应用窗口 |
| Tauri windowEffects（macOS 映射到 NSVisualEffectView） | 让**窗口本身**获得系统材质，可采样窗口背后的桌面 | 只能整窗设置，无法对页面内某个 div 单独设置 |

**结论**：想要“侧边栏真的透出桌面”，必须用 Tauri 的 windowEffects（窗口级），并让网页侧边栏**透明**；想要“内容滚动时工具栏下方出现毛玻璃”，用 backdrop-filter（页面级）。两者可叠加，但要注意别做两遍模糊。

Tauri 2 的 Effect 枚举（v2.11.x，实测自 docs.rs）在 macOS 上可用值，与 §1.3 的 AppKit 材质一一对应：

~~~
Sidebar, HeaderView, UnderWindowBackground, UnderPageBackground,
HudWindow, Menu, Popover, Selection, Sheet, Titlebar, Tooltip,
ContentBackground, FullScreenUI, WindowBackground
（Windows 专属：Blur / Acrylic / Mica / MicaDark / MicaLight / Tabbed*；
  已废弃勿用：AppearanceBased / Light / Dark / MediumLight / UltraDark）
~~~

配置形态（tauri.conf.json）：

~~~json
{
  "app": {
    "windows": [{
      "label": "main",
      "transparent": true,
      "windowEffects": {
        "effects": ["sidebar"],
        "state": "followsWindowActiveState",
        "radius": 10
      }
    }]
  }
}
~~~

注意：transparent: true 是前置条件；state 设为 followsWindowActiveState 可复刻“非 key 窗口材质退场”的原生行为（即 §8 第 3 条）。

## 10.2 CSS 变量层（:root）

~~~css
:root {
  /* ——— 系统色（HIG 统一色板，2025-06 值）——— */
  --sys-blue:    #0088FF;  --sys-red:     #FF383C;
  --sys-orange:  #FF8D28;  --sys-yellow:  #FFCC00;
  --sys-green:   #34C759;  --sys-mint:    #00C8B3;
  --sys-teal:    #00C3D0;  --sys-cyan:    #00C0E8;
  --sys-indigo:  #6155F5;  --sys-purple:  #CB30E0;
  --sys-pink:    #FF2D55;  --sys-brown:   #AC7F5E;
  --sys-gray:    #8E8E93;

  /* ——— 强调色（AccentColor，默认 = system blue）——— */
  --accent: var(--sys-blue);
  --accent-contrast: #FFFFFF;

  /* ——— 语义色 ——— */
  --label:              rgba(0, 0, 0, 0.85);
  --label-secondary:    rgba(0, 0, 0, 0.50);
  --label-tertiary:     rgba(0, 0, 0, 0.26);
  --label-quaternary:   rgba(0, 0, 0, 0.10);
  --label-placeholder:  rgba(0, 0, 0, 0.25);
  --label-disabled:     rgba(0, 0, 0, 0.20);
  --separator:          rgba(0, 0, 0, 0.10);

  /* ——— 容器 ——— */
  --window-bg:          #ECECEC;
  --content-bg:         #FFFFFF;
  --control-bg:         #FFFFFF;
  --selected-content:   #0063E1;
  --unemphasized-selected: rgba(0, 0, 0, 0.06);
  --selected-text-bg:   #B3D7FF;

  /* ——— 材质 ——— */
  --material-sidebar-bg:  rgba(246, 246, 246, 0.72);
  --material-toolbar-bg:  rgba(246, 246, 246, 0.62);
  --material-menu-bg:     rgba(246, 246, 246, 0.82);
  --material-blur:        blur(30px) saturate(180%);
  --material-blur-light:  blur(24px) saturate(180%);
  --material-rim:         inset 0 0 0 0.5px rgba(255,255,255,0.35),
                          0 0 0 0.5px rgba(0,0,0,0.12);

  /* ——— 形状 ——— */
  --radius-window:  20px;   /* Tahoe 窗口近似（真实值随工具栏高度变化） */
  --radius-panel:   12px;
  --radius-control: 6px;
  --radius-list-row: 6px;

  /* ——— 间距与控件 ——— */
  --control-height:      22px;
  --control-height-sm:   19px;
  --control-height-mini: 15px;
  --toolbar-height:      52px;
  --sidebar-width:       240px;
  --sidebar-min-width:   200px;
  --sidebar-max-width:   320px;
  --row-height:          28px;
  --content-padding:     20px;

  /* ——— 排版 ——— */
  --font-ui: -apple-system, BlinkMacSystemFont, "SF Pro Text", "PingFang SC",
             "Hiragino Sans GB", "Microsoft YaHei", system-ui, sans-serif;
  --font-mono: "SF Mono", ui-monospace, "Cascadia Mono", Consolas, monospace;

  --text-largetitle: 26px;  --leading-largetitle: 32px;
  --text-title1: 22px;      --leading-title1: 26px;
  --text-title2: 17px;      --leading-title2: 22px;
  --text-title3: 15px;      --leading-title3: 20px;
  --text-headline: 13px;    --leading-headline: 16px;
  --text-body: 13px;        --leading-body: 16px;
  --text-callout: 12px;     --leading-callout: 15px;
  --text-subheadline: 11px; --leading-subheadline: 14px;
  --text-footnote: 10px;    --leading-footnote: 13px;
  --text-caption1: 10px;    --leading-caption1: 13px;

  /* ——— 字距（HIG tracking 表）——— */
  --tracking-body: -0.08px;    /* 13pt */
  --tracking-title2: -0.43px;  /* 17pt */
  --tracking-title1: -0.26px;  /* 22pt */
  --tracking-large: 0.22px;    /* 26pt */

  /* ——— 动效 ——— */
  --dur-hover: 120ms;
  --dur-press: 80ms;
  --dur-state: 180ms;
  --dur-popover: 200ms;
  --dur-panel: 280ms;
  --ease-standard: cubic-bezier(0.25, 0.1, 0.25, 1);
  --ease-out: cubic-bezier(0.2, 0, 0, 1);
  --ease-panel: cubic-bezier(0.32, 0.72, 0, 1);
}

/* 深色模式：跟随系统，不提供应用级开关 */
@media (prefers-color-scheme: dark) {
  :root {
    --sys-blue: #0091FF;   --sys-red: #FF4245;
    --sys-orange: #FF9230; --sys-yellow: #FFD600;
    --sys-green: #30D158;  --sys-mint: #00DAC3;
    --sys-teal: #00D2E0;   --sys-cyan: #3CD3FE;
    --sys-indigo: #6D7CFF; --sys-purple: #DB34F2;
    --sys-pink: #FF375F;   --sys-brown: #B78A66;
    --sys-gray: #8E8E93;

    --label:              rgba(255, 255, 255, 0.85);
    --label-secondary:    rgba(255, 255, 255, 0.50);
    --label-tertiary:     rgba(255, 255, 255, 0.25);
    --label-quaternary:   rgba(255, 255, 255, 0.10);
    --label-placeholder:  rgba(255, 255, 255, 0.25);
    --label-disabled:     rgba(255, 255, 255, 0.20);
    --separator:          rgba(255, 255, 255, 0.10);

    --window-bg:          #323232;
    --content-bg:         #1E1E1E;
    --control-bg:         #1E1E1E;
    --selected-content:   #0058D0;
    --unemphasized-selected: rgba(255, 255, 255, 0.08);
    --selected-text-bg:   #3F638B;

    --material-sidebar-bg: rgba(30, 30, 30, 0.62);
    --material-toolbar-bg: rgba(28, 28, 28, 0.55);
    --material-menu-bg:    rgba(40, 40, 40, 0.82);
    --material-rim:        inset 0 0 0 0.5px rgba(255,255,255,0.12),
                           0 0 0 0.5px rgba(0,0,0,0.45);
  }
}

/* 无障碍降级：Reduce Transparency → 不透明回退 */
@media (prefers-reduced-transparency: reduce) {
  :root {
    --material-sidebar-bg: #F2F2F2;
    --material-toolbar-bg: #F2F2F2;
    --material-menu-bg:    #F6F6F6;
    --material-blur: none;
    --material-blur-light: none;
  }
}
@media (prefers-reduced-transparency: reduce) and (prefers-color-scheme: dark) {
  :root {
    --material-sidebar-bg: #262626;
    --material-toolbar-bg: #232323;
    --material-menu-bg:    #2C2C2C;
  }
}

/* 无障碍降级：Reduce Motion → 去掉位移与过渡 */
@media (prefers-reduced-motion: reduce) {
  :root {
    --dur-state: 100ms;
    --dur-popover: 100ms;
    --dur-panel: 100ms;
  }
  .anim-slide, [data-anim="slide"] { transform: none !important; }
}
~~~

## 10.3 Tailwind v4 @theme 约定

Tailwind v4 是 CSS-first 配置，把上面的变量映射进 @theme 即可获得工具类：

~~~css
@import "tailwindcss";

@theme {
  /* 颜色：自动生成 bg-mac-blue / text-mac-label-secondary 之类 */
  --color-mac-blue:      var(--sys-blue);
  --color-mac-red:       var(--sys-red);
  --color-mac-green:     var(--sys-green);
  --color-mac-orange:    var(--sys-orange);
  --color-mac-gray:      var(--sys-gray);
  --color-mac-label:     var(--label);
  --color-mac-label-2:   var(--label-secondary);
  --color-mac-label-3:   var(--label-tertiary);
  --color-mac-separator: var(--separator);
  --color-mac-content:   var(--content-bg);
  --color-mac-window:    var(--window-bg);
  --color-mac-accent:    var(--accent);

  /* 圆角 */
  --radius-mac-control: 6px;
  --radius-mac-panel:   12px;
  --radius-mac-window:  20px;

  /* 字号 + 行高（Tailwind v4 支持 --text-* / --text-*--line-height） */
  --text-body:    13px;
  --text-body--line-height: 16px;
  --text-title3:  15px;
  --text-title3--line-height: 20px;
  --text-title2:  17px;
  --text-title2--line-height: 22px;
  --text-title1:  22px;
  --text-title1--line-height: 26px;

  /* 间距 */
  --spacing-mac-content: 20px;
  --spacing-mac-row:     28px;
  --spacing-mac-control: 22px;

  /* 动效 */
  --ease-mac-panel: cubic-bezier(0.32, 0.72, 0, 1);
  --duration-mac-state: 180ms;
}

/* 组件层：材质工具类 */
@utility material-sidebar {
  background-color: var(--material-sidebar-bg);
  -webkit-backdrop-filter: var(--material-blur);
  backdrop-filter: var(--material-blur);
  box-shadow: var(--material-rim);
}
@utility material-toolbar {
  background-color: var(--material-toolbar-bg);
  -webkit-backdrop-filter: var(--material-blur-light);
  backdrop-filter: var(--material-blur-light);
}
@utility material-menu {
  background-color: var(--material-menu-bg);
  -webkit-backdrop-filter: var(--material-blur);
  backdrop-filter: var(--material-blur);
  box-shadow: var(--material-rim);
}
/* 不支持 backdrop-filter 的引擎：退化为不透明 */
@supports not ((backdrop-filter: blur(1px)) or (-webkit-backdrop-filter: blur(1px))) {
  .material-sidebar, .material-toolbar, .material-menu { background-color: var(--window-bg); }
}
~~~

推荐的组件约定（与 §6 对应）：

~~~
按钮   : h-[22px] rounded-[6px] text-[13px] leading-[16px]
主按钮 : bg-mac-accent text-white        （一屏仅 1 个）
次按钮 : bg-mac-control border border-mac-separator
危险   : text-mac-red bg-transparent
侧边栏 : w-[240px] material-sidebar  + 选中块 mx-[8px] rounded-[6px] bg-mac-accent text-white
工具栏 : h-[52px] material-toolbar   + 图标按钮 size-[28px] 无边框，hover:bg-black/5
列表行 : h-[28px] 交替行色 rgba(0,0,0,0.024)
分隔线 : h-px bg-mac-separator
~~~

## 10.4 backdrop-filter 的用法与性能边界

**用法要点**

1. **只挂在固定尺寸的栏上**（sidebar / toolbar / popover / menu），不要挂在滚动容器、长列表项或整页上。
2. 模糊半径控制在 **≤ 30px**（blur(30px) 已是原生观感上限）。半径与像素面积共同决定开销，半径翻倍，采样成本按面积增长。
3. 必须同时写 -webkit-backdrop-filter（WKWebView / Safari 仍需要前缀）。
4. 玻璃元素**不要嵌套多层**；多层 backdrop-filter 会互相采样，开销急剧上升（这与 macOS 一致：原生只用一层 backdrop + 一层 tint）。
5. 内容滚过玻璃时玻璃层重绘是必然的 —— 把滚动内容与玻璃层放入不同的合成层（玻璃层加 isolation: isolate），避免整页重排。
6. will-change: backdrop-filter **只在动画期间临时加**，用完移除；常驻会长期占用显存。
7. **不要对玻璃层做透明度动画**（会触发滤镜链重算）：要淡入淡出就淡在**不透明遮罩**上。
8. 亮背景之上仍要保证对比时，按官方做法加一层 **35% 不透明度的暗色遮罩**，而不是无限调高 blur。

**性能边界（实测经验 + 平台约束）**

| 约束 | 说明 |
| --- | --- |
| WKWebView（macOS） | 支持 backdrop-filter；但“窗口透明 + 大面积玻璃 + 高刷滚动”是最容易掉帧的组合 |
| WebView2（Windows） | 基于 Chromium，支持良好；模糊走 GPU 采样，最大化窗口 + 大面积玻璃在集显上会明显掉帧 |
| 与 Tauri windowEffects 叠加 | 窗口级 vibrancy 已是“整窗一次采样”；网页内再覆盖大面积 backdrop-filter 等于做两遍模糊 |
| 最坏情况 | 3000×1200 的动画滚动内容上挂 3 层 backdrop-filter，集显机器可掉到 30fps 以下 |
| 安全实践 | 玻璃总面积 ≤ 窗口面积的 25%（240px 侧边栏 + 52px 工具栏 ≈ 22%），模糊半径 20–30px，玻璃层不参与滚动重绘 |

**降级顺序（应当实现）**

~~~
支持 backdrop-filter → 半透明 + 模糊（+ 必要时不透明遮罩）
不支持               → 不透明纯色（--window-bg）
Reduce Transparency  → 不透明纯色
Reduce Motion        → 只保留淡入淡出，去掉形变位移
~~~

## 10.5 窗口与原生集成的对应关系

| macOS 原生概念 | Tauri 2 对应 | 备注 |
| --- | --- | --- |
| NSVisualEffectView.Material | windowEffects.effects: ["sidebar" / "headerView" / …] | 见 §10.1 枚举 |
| key / inactive 窗口材质差异 | windowEffects.state: "followsWindowActiveState" | 复刻 §8 第 3 条 |
| 窗口圆角 | 由系统控制（Tauri 不暴露）；windowEffects.radius 只影响效果裁剪半径 | 自绘窗口时需自己对齐 |
| 交通灯 | decorations: true（或 macOS 的 overlay 标题栏） | 不要自绘 |
| 全局菜单栏 | tauri::menu / MenuBuilder（macOS 上映射为 NSMenu） | §6.8 的必备菜单结构用它实现 |
| 工具栏 | 无原生映射 → HTML 自绘 52px 高栏 | 需自行保证无边框图标与 hover 态 |
| 侧边栏 | 无原生映射 → HTML + material-sidebar，或用 windowEffects 让整窗玻璃 | 二选一，避免双重模糊 |

---

# 11. 来源清单

## Apple 官方（现行）

1. HIG — Materials
2. HIG — Color
3. HIG — Typography
4. HIG — Layout
5. HIG — Sidebars
6. HIG — Toolbars
7. HIG — Buttons
8. HIG — The menu bar
9. HIG — Windows
10. HIG — Lists and tables
11. HIG — Menus
12. HIG — Toggles
13. HIG — Sliders
14. HIG — Segmented controls
15. HIG — Pop-up buttons
16. HIG — Motion
17. HIG — Accessibility
18. HIG — Dark Mode
19. HIG — Icons
20. HIG — SF Symbols
21. HIG — Split views
22. HIG — Scroll views
23. HIG — Tab views
24. Technology Overviews — Liquid Glass
25. Technology Overviews — Adopting Liquid Glass
26. AppKit — NSVisualEffectView.Material（材质枚举与用途）
27. AppKit — NSColor（UI element colors 表）
28. Apple 官方色板资源（developer.apple.com/tutorials/images/com.apple.HIG/*.png，72 张，用于提取 hex）
29. Apple Developer — Design Resources（macOS UI Kit 入口）

## Apple 官方（早期，存档）

30. Apple Human Interface Guidelines — Controls（Mac OS X 时期存档镜像）：控件高度 22 / 19 / 15 px

## 转引的 Apple 一手内容

31. WWDC 2025 Session 310 “Build an AppKit app with the new design”（圆角随窗口样式变化、concentricity、LayoutRegion）—— 经 lapcatsoftware.com 转引
32. Apple Newsroom “Apple introduces a delightful and elegant new software design”（concentric 表述）—— 经 mjtsai.com 转引

## 第三方（已在正文标注）

33. Oskar Groth — Reverse Engineering NSVisualEffectView（层栈、滤镜链、backdrop scale、rim）
34. Gerald Versluis — iOS and macOS Dark Mode Dynamic Colors Overview（AppKit 语义色 RGBA）
35. Jeff Johnson / lapcatsoftware.com — macOS Tahoe windows have different corner radiuses；The evolution of Mac app window corners
36. MacRumors Forums — 窗口圆角实测值（私有 defaults 键 NSConvolutionOverride1：10 / 20 / 26）
37. Tauri — tauri::window::Effect 枚举（docs.rs，v2.11.5）
38. Mario Guzman — Sidebar Guidelines（侧边栏宽度建议）

> 未采信：ubos.tech 的 “macOS Tahoe Liquid Glass UI Review”（内容为 AI 拼接，其“Sequoia 4px → Tahoe 12px”与实测矛盾，已剔除）。

## 不确定项（明确标注，勿当规范使用）

1. **macOS 26 的窗口圆角 26pt**：来自社区对私有 defaults 键的实测，Apple 未公开；“带工具栏 / 仅标题栏”两档的具体差值只有定性描述。
2. **AppKit 语义色的 alpha**：Apple 从不公开，第三方在 0.8 与 0.85（label）、0.2 与 0.26（tertiary）之间存在分歧；建议按视觉效果微调。
3. **材质的 blur / saturation / brightness 参数**：来自逆向工程示例，不是 Apple 的材质参数；§1.4 的 CSS 等效配方是本规范的工程近似。
4. **Tahoe 的控件圆角、行高、工具栏高度绝对值**：官方只说“更大”，无公开数值。
