# AudioLink 设计规范与样张

这个目录装的是**两套平台设计语言的权威规范**和它们的**可运行样张**。
产品代码在 `desktop/`（Tauri + React + Tailwind v4）与 `android/`（Compose），本目录只放设计依据与预览。

## 目录内容

| 文件 | 是什么 | 依据 |
|---|---|---|
| `fluent-2.md` | **Microsoft Fluent 2 / Windows 11** 规范要素（78 KB） | 43 项微软官方来源：fluent2.microsoft.design 9 页 + learn.microsoft.com 11 页 + WinUI 3 主题资源源码 8 个 + Fluent UI tokens 源码 15 个 |
| `macos-hig.md` | **Apple macOS** 规范要素（58 KB） | 30 项 Apple 官方来源：23 个 HIG 页面（走官方 JSON 数据端点取的原文）+ 2 个 Technology Overviews + 2 个 AppKit API 参考 + 72 张官方色板 PNG（颜色为像素取样，非二手转抄） |
| `preview/fluent.html` | Fluent 语言的**单文件样张** | 对照 `fluent-2.md` 逐条落位 |
| `preview/macos.html` | macOS 语言的**单文件样张** | 对照 `macos-hig.md` 逐条落位 |
| `.raw/` | 规范调研的原始抓取物、官方 JSON、色板 PNG | 供复核，勿删 |

两份规范都带**「给 Tauri/React 的落地映射」**一节（CSS 变量命名、Tailwind v4 `@theme` 约定、组件工具类清单、自检清单）——实现时直接照那一节抄。

## 怎么打开样张

直接双击 `preview/*.html` 即可（单文件、零外部依赖、自带深浅主题开关）。

或者起本地预览服务（浏览器不允许直接开 `file://` 时用）：

```powershell
cd docs/design/preview
node _serve.cjs        # 监听 http://127.0.0.1:8799/
```

## 两套语言的**关键分野**（这是它们不像的原因，也是它们不该像的原因）

| 维度 | Fluent（Windows 11） | macOS |
|---|---|---|
| 玻璃范围 | **多层**：窗口底 Mica + 卡片本身也是玻璃片（壁纸→Mica→侧栏→卡片→浮层五层） | **只在导航层**：侧栏/工具栏是 vibrancy，**内容面板不透明**（Apple 明令内容层不用玻璃） |
| 窗口结构 | 内容全宽铺在 Mica 底上，每块自成一枚 Card | 内容是一整块**浮起面板**（四周留缝 + 圆角 + 外投影） |
| 卡片 | 1px 描边 + 底边深一档（Windows 用 stroke 代替阴影） | 无边框浅灰分组块（System Settings 的 inset grouped） |
| 控件高度 | 32px | **22–24px**（官方口径） |
| 圆角 | 控件 4px / 浮层 8px | 控件 6px / 卡片 10px / 面板 12px；**同心圆角** |
| 标题 | Subtitle 20px + 下方 1px 分隔线 | **Large Title 26 / 700** |
| 焦点 | **内外双层描边**（外白内黑） | 强调色 **3px 半透明光晕** |
| 遮罩 | Smoke 纯色 `rgba(0,0,0,.302)` | backdrop blur |
| 字阶 | 12 / 14 / 20 | 11 / 13 / 15 |
| accent | `#0F6CBD` / 深 `#4CC2FF` | `#007AFF` / 深 `#0A84FF` |
| 一屏实心按钮 | 可以有多枚 accent 按钮 | **只允许一枚**（其余灰底描边） |

## 玻璃为什么会"死"（踩过的坑，别再踩）

`backdrop-filter` 采样的是**它身后的像素**。下面三件事任一发生，玻璃就完全失效、页面塌成一块灰板：

1. 底下的祖先元素是**不透明纯色**（没有可透之物）；
2. 材质挂在**将要被子元素采样**的那一层自身上（后代采样不到它）；
3. 中间夹着**不透明面板**（例如工具栏被包在 panel 里面）。

正确结构：**独立的 fixed 壁纸层（z0）→ 半透明 + blur 的材质层（z1）→ 不透明内容层（z2）**，层与层之间不要互相遮挡。

## 实施红线（2026-09-18 实测得出，落地时必须遵守）

### 1. 浮层一律脱离玻璃祖先（React 里 = 必须用 portal）

带 `backdrop-filter` 的元素会成为两样东西，两个都会咬人：

- **后代的 backdrop root**：后代浮层能采样到的背景被**限制在这个祖先的盒子内**。浮层伸出盒子的部分没有内容可采样 → `blur()` **完全失效**，只剩半透明底（表现为"底下文字清晰透过"）。
- **`fixed` 后代的 containing block**：探针实测，浮层留在玻璃祖先内改 `position: fixed`，写 `left:300px` 实际渲染在 `580px`（祖先左缘 280 + 300）。**只改定位、不移 DOM 是死路。**

做法：浮层的 DOM 挂在 `<body>` 直接子级，`position: fixed` + 用触发元素的 `getBoundingClientRect()` 算坐标，并在 `resize` / `scroll`（捕获阶段）重算。
React 落地对应：**一律 `createPortal` 挂 `body`**，不允许就地渲染在工具栏组件里。

### 2. portal 要配焦点转移

浮层挂到 `body` 末尾后，Tab 从触发按钮进浮层会绕到页面底部。必须配套：打开时程序化聚焦浮层内第一个可聚焦元素，关闭时把焦点**归还给触发按钮**。

### 3. 玻璃不要嵌套

同一块内容被两层 `backdrop-filter` 叠加 = 模糊两次（当前样张里 `sidebar(blur32)` ⊃ `collect-card(blur16)` 就是这样，落地时去掉卡片那层）。按 Fluent 规范：**Acrylic 只属于浮层**；卡片/面板用 `Layer`/`Card` 的**填充**表达层次，不需要自己的 backdrop。

### 4. 玻璃要"糊得住、透得出"

只做半透明 = 雾贴纸，不是玻璃。判据：壁纸色到达该层的比例（"累计透光率"）落在 **10–15%**（当前四层分别是 11.5% / 13.9% / 2.2%）；调完必须验一条——**背景切成单色时整个 UI 要被明显染色**，否则就是糊成了灰板。

## 当前状态

- 规范：已完成，含实现映射与自检清单。
- 样张：两版均已完成，深/浅主题都验过渲染。
- **设计系统已固化**：项目根 `DESIGN.md`（YAML 令牌 + 八节规范 + 迁移清单）是唯一权威，本目录的规范稿是它的上游依据、样张是它的视觉基准。
- **待定**：壁纸饱和度（Fluent 版浅色偏甜，是否收敛一档）、`page-title` 最终档位、Android 是否保留自研 On-Air Console 暖调个性。
- 进行中：Android 端主题色对齐基准（已改 `ui/theme/`）；桌面端 `desktop/src` 的材质与浮层重构见 `DESIGN.md` Migration & Gaps。
