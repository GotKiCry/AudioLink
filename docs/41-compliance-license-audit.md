# M5 · 依赖许可审计（合规清单的第一块）

> ⚠️ **历史记录说明（2026-09-22 补）**：本文记录的是 **PIN 配对 / 信任库时代**的交付事实与证据。
> 该认证机制已在本轮整体移除（现在连上即用：没有配对码、没有白名单），本文中的配对步骤、PIN 验收数字与
> 信任判定均**不再对应当前实现**，只作为当时的交付记账保留 —— 现状见 `docs/72-remove-pairing.md`。

> 日期 **2026-09-16**。一句话：发布前必须回答「这些依赖允许我这样分发吗」，
> 而这轮把它做成了**可重复跑 + 有 CI 护栏**的东西，而不是一次性的人工核查。

---

## 1. 为什么分两半

| 一半 | 位置 | 为什么在那里 |
|---|---|---|
| **采集** | `tools/license-audit.ps1` | 要会调 `cargo metadata` 与 `pnpm licenses` —— 这是**环境知识**（工具怎么装、输出什么形状） |
| **判定 + 渲染** | `audiolink-tools::license`（Rust） | 判定逻辑必须能**离线单测**：13 项单测覆盖 OR/AND/WITH、旧式 `/`、括号、未知许可以及报告幂等 |

CLI：`cargo run -q -p audiolink-tools --bin license-audit -- --cargo <json> [--npm <json>] [--write <report.md>]`。
退出码 0 = 无禁止许可；1 = 有（发布前必须处理）；2 = 用法/解析错误。

## 2. 判定语义（SPDX）

| 连接词 | 语义 | 处置 |
|---|---|---|
| `A OR B` | 任一可用即可（**选择权在我们**） | 取最宽松的一项 |
| `A AND B` | 义务叠加，全部必须可用 | 取最严格的一项 |
| `A WITH B` | 例外会改写义务 | 把 `X WITH Y` 当**一个原子**（白名单里显式列出） |

三条**刻意的保守规则**：

1. **未知许可进 `notice`，不进 `denied`** —— 未知不等于安全，也不等于不能用；当允许会漏，当禁止会误杀。
2. **`AND` 与 `OR` 混用的表达式一律进 `notice`** —— 完整 SPDX 优先级（括号/优先级）不在本审计的范围里，
   与其猜，不如交给人看。实测里 `unicode-ident` 的 `(MIT OR Apache-2.0) AND Unicode-3.0` 就是这一档。
3. 旧式写法 `MIT/Apache-2.0`（用 `/` 表示 OR）按历史约定当 `OR` —— 数据里真实存在。

## 3. 实测（首次运行）

| 来源 | 包数 | allowed | notice | denied |
|---|---|---|---|---|
| Rust（`cargo metadata`，含本 workspace） | 635 | 620 | 15 | **0** |
| 前端（`pnpm licenses`） | 88 | 86 | 2 | **0** |

报告落盘在 `docs/compliance/license-report.md`（**自动生成、幂等**：不含时间戳与路径，CI 里跑不会产生 diff）。

### 3.1 报告暴露出的真实待办（这就是这块工作的价值）

| 发现 | 影响 |
|---|---|
| **`uniffi` 全家 8 个包是 MPL-2.0** | 文件级 copyleft：可以静态链接进闭源产品，但**被修改过的 MPL 文件必须公开**。我们没改过 uniffi 源码 → 当前合规，但这条要写进发布检查单 |
| `lightningcss`（前端构建工具链）2 个包 MPL-2.0 | 只进构建，不进分发产物 → 义务更轻 |
| `webpki-root-certs` 是 `CDLA-Permissive-2.0` | 宽松许可但不在白名单 → 进 `notice`（**没有自动放行，这是对的**） |
| `unicode-ident` 的 `(MIT OR Apache-2.0) AND Unicode-3.0` | 混用表达式 → `notice`；实际两项都宽松，但审计不做优先级推断 |

**没有 GPL / AGPL / SSPL / 非商业许可** —— 这是「可以发布」的前提结论，而且它是**可重复验证**的，不是一次性的。

## 4. CI 护栏

`desktop` job 新增一步（那里同时有 `cargo` 与 `pnpm`，两边都能采到）：

```yaml
      - name: License audit (Rust + frontend)
        run: pwsh tools/license-audit.ps1
```

意义：**新引入一个不能分发的依赖会在 CI 里红**，而不是等到发布前才发现。

## 5. 覆盖面（诚实清单）

- **已覆盖**：Rust workspace 的全部依赖、桌面前端依赖；
- **未覆盖**：~~Android（Gradle）依赖、~~随包分发的二进制内部第三方库、字体与图标资源（Android 依赖已在 §8 补上，见 §10）；
- 本报告回答的是「许可是否允许这样分发」；**署名/免责文本的实际投放位置是另一件事**（~~还没做~~ → **已做**：桌面 §7、Android §9，见 §10）。

---

## 6. 第三方组件声明（2026-09-16 第二轮）

§5 里"署名/免责文本的投放位置"这一条，本轮把**材料**做出来了：`THIRD-PARTY-NOTICES.md`。

| 内容 | 说明 |
|---|---|
| 组件清单 | 按**许可原文**分组（635 个包里剔除 10 个自有 crate = 625 个第三方组件） |
| 许可全文 | 从每个包的 `manifest_path` **同目录**读 `LICENSE*` / `LICENCE*` / `COPYING*` / `NOTICE*`，按**内容去重** → **324 份** |
| 缺全文标记 | 只在 `Cargo.toml` 里写了 SPDX 标识的包显式标 `⚠️无全文`，不假装有 |

生成命令：`pwsh tools/license-audit.ps1 -Notices`（平时不生成 —— 它 1 MB，只在发布前需要）。

### 6.1 两个刻意的口径选择

**全量口径**：清单包含**整棵依赖图**（含构建期依赖），它是分发包实际内容集的**超集**。
多列不构成合规问题，漏列才是 —— 收窄到"运行时子集"是可选优化，不是必需。

**自有 crate 不进第三方声明**：`cargo metadata` 里 `source` 为 null 的包就是本仓库自己的，
它们不是"第三方"；而且它们的许可文件在仓库根（`LICENSE` + `NOTICE` 都在），不在各自目录下 ——
第一版把它们列进去并标了"⚠️无全文"，是**分类错误**，已修（组件数 635 → 625）。

### 6.2 还差什么

- **投放位置**：声明文件躺在仓库里 ≠ 用户能看见。放进安装包 / "关于"页展示是发布流程的一步；
- ~~Android（Gradle）依赖仍未进清单（见 §5 的覆盖面清单）。~~ → **已在 §8 补上（114 个组件）**，见 §10。

---

## 7. 投放：让用户看得见（2026-09-16 第三轮）

§6.2 记的「声明躺在仓库里 ≠ 用户看得见」，本轮做完。三处改动：

| 位置 | 改动 |
|---|---|
| 打包 | `bundle.resources` 收录 `docs/compliance/THIRD-PARTY-NOTICES.md` —— 安装后它落在**资源目录** |
| 命令 | `third_party_notices` 依次找「资源目录」→「仓库生成路径（开发版）」，返回 来源 / 字节数 / 全文 |
| 界面 | 「关于 / 第三方声明」面板：懒加载（点击才拉，它 1 MB），可滚动的全文 |

**找不到时不说「文件不存在」**：返回的是「在仓库根执行 `pwsh tools/license-audit.ps1 -Notices`」——
空面板只会让人以为软件坏了。这条有单测钉住（`notices_view(None)` 必须含生成命令）。

**打包前置条件**：`-Notices` 没跑过就 `tauri build` 会因为找不到资源文件失败 —— 这是有意的，
宁可打不出包，也不要打出一个没有声明的包。

单测 2 项（找到时报出来源与字节数、找不到时给生成提示）。

### 7.1 还差什么

- **真实安装包验证**：本轮只到「资源被收录 + 命令能读到 + 界面能显示」这一层，
  `tauri build` 出包后在干净机器上打开「关于」还没验过（**第 110 轮复核仍成立**，见 §10）；
- ~~**Android 侧没有对应投放**：Android 的分发物里同样需要声明，且 Gradle 依赖还没进清单（§5）。~~
  → **已在 §9 补上**（声明随 APK 分发 + 应用内「开源许可」页；Gradle 依赖见 §8），见 §10。

---

## 8. Android（Gradle/Maven）依赖纳入审计（2026-09-16 第四轮）

§5 与 §7.1 里反复出现的「Android Gradle 依赖未覆盖」，这一轮补上了。

| 环节 | 做法 | 为什么这样做 |
|---|---|---|
| 采集依赖坐标 | `gradlew :app:dependencies --configuration releaseRuntimeClasspath` 的依赖树 | 不引入第三方 Gradle 许可插件（那要往构建里再塞一个供应链依赖，还要联网） |
| 读许可 | Gradle 缓存里每个 POM 的 `<licenses>` 节点 | 完全离线；缓存本来就为构建下过 |
| 规范化 + 判定 | `audiolink-tools::license`（Rust，可单测） | POM 写的是**自然语言名**（`Apache License, Version 2.0`），要先变成 SPDX 才谈得上判定 |

### 8.1 实测（首次）

```text
license-audit: Rust 635 包 / 前端 88 包 / Android 114 包 · denied 0 · notice 17
```

Android 一节：**114 个组件，allowed 113 / notice 1 / denied 0**。那一条 `notice` 是
`com.google.guava:listenablefuture 1.0` —— 它的 POM **没有声明任何许可**（著名的空壳 POM，
真正的许可由 `guava` 本体覆盖）。它进 `notice` 而不是被当成 allowed，正是「未知不等于安全」这条规则在起作用。

许可名分布：110 个 `The Apache Software License, Version 2.0` + 10 个 `Apache-2.0` +
1 个 `BSD-3-Clause` + 1 个 `LGPL-2.1-or-later OR Apache-2.0`（JNA 双许可 → `OR` 取最宽 → allowed）+ 1 条未声明。

### 8.2 两个刻意的细节

**规范化顺序有语义**：`lesser general public license` 必须排在 `general public license` 之前，
否则 LGPL 会被误判成 GPL —— 一个可分发、一个不可分发。这条有单测钉住（`lgpl_is_not_mistaken_for_gpl`）。

**`(c)` 是版本约束不是依赖**：Gradle 树里带 `(c)` 的行只表示"约束版本"，把它当依赖会把组件数虚高。

### 8.3 还差什么

- ~~**Android 侧没有投放位置**：桌面端有「关于」面板读声明（§7），Android 应用内还没有对应入口；~~
  → **已在 §9 补上**，见 §10；
- **CI 未接**：Android 许可审计目前是「发布前手动跑」的一步，还没进 workflow（桌面侧的审计在 `desktop` job 里）。
  **第 110 轮复核仍成立，且依据更精确**：`ci.yml` 的 `License audit (Rust + frontend)` 步骤确实在，
  但它不跑 `tools/android-licenses.ps1` → `target/evidence/compliance/android-licenses.json` 不存在时脚本只打印
  「未找到 Android 依赖清单，本次跳过 Android」；`release.yml` 的 `Generate third-party notices` 同样不带 Android 采集。见 §10。

---

## 9. Android 侧的声明投放（2026-09-16 第五轮）

§8.3 记的「Android 应用内没有声明入口」，这一轮补上。

| 环节 | 做法 |
|---|---|
| 打进包 | `license-audit.ps1 -Notices` 生成后**同步一份**到 `android/app/src/main/assets/`（只有一个生成源，手工复制迟早会漂移） |
| 读取 | `NoticesLoader.fromText(...)` —— 纯函数，把「读到的内容」折成界面要的形状 |
| 界面 | `LicensesScreen`（Compose）：从 assets 读、可滚动；入口是主界面顶栏的「开源许可」 |
| 找不到时 | 与桌面端同一条规则：**说清怎么生成**，不显示空白页 |

### 9.1 实测

```text
> Task :app:testDebugUnitTest :app:assembleDebug
BUILD SUCCESSFUL in 55s
```

| 核对项 | 结果 |
|---|---|
| 单测 | `NoticesLoaderTest` **3 项通过**（另有 9 个既有测试类全绿：共 76 项） |
| APK 内容 | `7z l app-debug.apk` → **`assets\THIRD-PARTY-NOTICES.md`**（1 056 775 B，压缩后 179 045 B） |
| APK 体积 | 19.17 MB（声明约占 180 KB 压缩后） |

### 9.2 一个刻意的设计：加载逻辑做成纯函数

读 assets 需要 `Context`（真机或仪器测试才能覆盖），而真正需要钉住的是**「找不到怎么办」这条规则** ——
它只在"打包漏了资源"时才走到，恰恰是最不容易在开发机上遇到的情况。所以 `fromText` 是纯函数，
三条单测全在纯 JVM 上跑，并且覆盖了「空文件与没有文件是同一件事」。

### 9.3 还差什么

- **真机验证**：本次验的是「资源进了 APK + 逻辑单测通过」，**没有**在真机上点开「开源许可」看它实际渲染；
- **CI 未接**：Android 许可审计仍是「发布前手动跑」；
- 桌面端的「关于」面板与 Android 这个页面还没有共享同一份文案常量（两边各自维护提示语）。

---

## 10. 第 110 轮口径复核（2026-09-17）

审计（`docs/53-m5-audit.md` §2-D）点名：「§5/§6 仍写 Android 依赖未进清单 / 没有声明的投放位置」。
逐句复核后，**过时的是四处**（已在原处划掉并指向本节），另有三处**仍然成立**（列在下面，不必再核）。

| 原表述 | 判定 | 今天实况 | 依据（符号名 / 可复核读数） |
|---|---|---|---|
| §5「未覆盖：Android（Gradle）依赖、…」 | **过时**（仅该半句） | Android 依赖**已覆盖**：114 个组件（allowed 113 / notice 1 / denied 0） | §8；采集器 `tools/android-licenses.ps1` → `tools/license-audit.ps1` 读 `target/evidence/compliance/android-licenses.json`；报告落点 `docs/compliance/license-report.md` 的「Android（Gradle/Maven）依赖」节 |
| §5「署名/免责文本的实际投放位置是另一件事（还没做）」 | **过时** | 两处投放都做了 | §7（桌面：`bundle.resources` 的 map 映射 + `third_party_notices` 命令 + 「关于」面板）、§9（Android：`assets/THIRD-PARTY-NOTICES.md` + `NoticesLoader` + `LicensesScreen`）|
| §6.2「Android（Gradle）依赖仍未进清单」 | **过时** | 同第一行 | §8；`docs/compliance/THIRD-PARTY-NOTICES.md` 内含 Android 组件（如 `com.google.guava:listenablefuture`）|
| §7.1「Android 侧没有对应投放」 | **过时** | 同第二行 | §9 |
| §7.1「真实安装包验证…干净机器上打开『关于』还没验过」 | **仍成立** | 验到的只是「资源进包 + 包内路径与运行时查找一致」 | §7.1；`docs/42-m5-release-pipeline.md` §4.2 |
| §8.3 / §9.3「CI 未接（Android 许可审计）」 | **仍成立，依据更精确** | `ci.yml` 的 `License audit (Rust + frontend)` 步骤在，但它**不**跑 `tools/android-licenses.ps1`；`release.yml` 的 `Generate third-party notices` 同样不带 Android 采集 → 两处 CI 环境下 Android 一栏都会「跳过」 | `.github/workflows/ci.yml`、`release.yml`；`tools/license-audit.ps1` 的 `-SkipAndroid` 分支与「未找到 Android 依赖清单，本次跳过 Android」提示 |
| §9.3「真机验证（没在真机上点开『开源许可』）」 | **仍成立** | 全仓无「真机点开许可页」的记录 | `docs/` 与 `target/evidence/` 下 grep「开源许可」只命中文档与手册 |
| §9.3「两边各自维护提示语」 | **仍成立** | 文案**相同**但**各自硬编码**：桌面 `view.rs` 的 `notices_view` 与 Android `NoticesLoader.MISSING_HINT` | `desktop/src-tauri/src/view.rs`；`android/app/src/main/kotlin/com/gotkicry/audiolink/compliance/NoticesLoader.kt` |

### 10.1 本轮顺带发现（**范围外**，未改，需另行处置）

1. `docs/compliance/license-report.md` 里的「**未覆盖**：…以及 Android 侧的**投放位置**（声明入口）」与
   「署名/免责文本的**实际投放位置**是另一件事」这两句是**自动生成的**，模板在
   `core/crates/audiolink-tools/src/license.rs` 的报告渲染函数里 —— **改文档没用，下次跑审计会把旧口径写回去**。
   本轮没动它（write scope 只有 `docs/41` 与 `docs/42`）。
2. `ci.yml` 里那一步的名字是 `License audit (Rust + frontend)`，与脚本现在的实际覆盖面（默认纳入 Android，
   只要清单 JSON 在）不符 —— 容易让人以为「CI 已覆盖 Android」。见 `docs/42` §13.1。
