# M5 产品化独立审计（task-30）

> ⚠️ **历史记录说明（2026-09-22 补）**：本文记录的是 **PIN 配对 / 信任库时代**的交付事实与证据。
> 该认证机制已在本轮整体移除（现在连上即用：没有配对码、没有白名单），本文中的配对步骤、PIN 验收数字与
> 信任判定均**不再对应当前实现**，只作为当时的交付记账保留 —— 现状见 `docs/72-remove-pairing.md`。

> 审计日期 **2026-09-17** ｜ 审计对象：看板 `[M5] 产品化：托盘/自启/自动更新/双语/双 ABI 发布/合规清单`（`tools/sync-board.ps1`，S = In Progress）
> 审计基准：**当前 HEAD 的工作区**（未改任何产品代码）；**定位一律用符号名**（函数 / 常量 / 文件名），只在文档章节处用 §号 —— 行号会漂，上次审计已经吃过一次。
> 上游：`docs/05-roadmap.md` §M5（交付物 5 条 + 验收 5 项）· `docs/41-compliance-license-audit.md` · `docs/42-m5-release-pipeline.md` · `docs/37`/`docs/46` 等子条目
> 方法：**逐项读当前 HEAD 的代码 + 实际跑门禁 + 能复核的数字一律复核**。本次没有采信任何看板/文档正文里的数字。

---

## 0. 结论先行

| 问题 | 结论 |
|---|---|
| 看板 [M5] 正文是否漏记？ | **是，漏记 4 项**：托盘、桌面开机自启、自动更新（含签名链路与安装包）、合规清单。正文 497 字符只提了双 ABI、双语、绿色版、用户文档 |
| 正文「至此只剩真机安装验收仍受阻」是否准确？ | **不准确**：它没提「真实更新流程验证」（面板另有 Todo）；此外 `docs/05` 的 M5 交付物 2/5 里还有**两个无人认领项**（Android 省电白名单引导、隐私说明） |
| 已认领子项的「实现」是否真在？ | **是**：8 项逐项都在代码里找到落点，且多数有自动化证据（见 §1） |
| `App.tsx` 的「不做：托盘、开机自启、双语」是否过时？ | **HEAD 版过时，工作区已修（未提交）**：HEAD 仍写「**不做**：托盘、开机自启、双语…（属 M5/M2）」；工作区已改成「**当时不做**（M1 范围）… 其中托盘 / 开机自启 / 双语**已在 M5 落地**…别再当成全局现状读」（`git diff` 3 增 1 删，**尚未提交**）——改后的措辞正确 |
| 能否按「实现 + 自动化证据齐备、真机/外部项已由独立条目承接」判 Done？ | **可以，但要先补两条**（见 §3）：把「Android 省电白名单引导」与「隐私说明」明确挂账或移出 M5 范围。否则 Done 会把两个未认领项永久掩盖 |

---

## 1. 逐项核对表

| 子项 | 实现落点（符号名） | 证据（测试名 / 可复核读数） | 结论 |
|---|---|---|---|
| **托盘** | `build_tray`（`desktop/src-tauri/src/lib.rs`）：`TrayIconBuilder::with_id("main")`，菜单两项 `show` / `quit`；`on_tray_icon_event` 左键唤出；`on_window_event` 里 `CloseRequested` → `prevent_close` + `hide`（关窗 ≠ 退出）；`quit_app` **先** `bridge.shutdown_engine()`（→ 内核给对方发 `BYE`）**再** `app.exit(0)` | **无自动化测试**（`audiolink-desktop` 40 项里没有 tray 相关）；属 UI/系统集成，只能人工/真机复核 | **成立（实现齐）／证据=人工读取** |
| **开机自启（桌面）** | `autostart_enabled` / `set_autostart`（`lib.rs`）+ `tauri_plugin_autostart::Builder`（装配点 `lib.rs` 的 `.plugin(...)`）+ 前端 `SettingsPanel.tsx` + `ipc.autostartEnabled/setAutostart` + i18n 键 `set.autostart`（zh/en 各一条） | **无自动化测试**：`SettingsPanel` 没有 `test.tsx`；rust 侧 `settings::tests::*`（4 项，PASS）测的是**启动时自动连接**，不是自启开关 | **成立（实现齐）／证据=人工读取** |
| **自动更新** | `update::check` / `update::install`（`desktop/src-tauri/src/update.rs`，安装必须带上用户看到的版本号 = 二次确认）；`tauri.conf.json` 的 `plugins.updater.endpoints` + `pubkey`；`capabilities/default.json` **故意不给 updater ACL**（注释写明：前端只调外壳自己的命令，给了就等于白送一条静默安装入口） | **跑过**：`updater_signature` **7 项 PASS**（含 4 类篡改拒绝 + `real_release_artifact_verifies_when_present`）、`updater_hosted_e2e::hosted_release_end_to_end_from_local_http` PASS、`update::tests::*` **4 项 PASS**；端到端安装仍需人工四步（`docs/42` §12.4）→ 面板 Todo 承接 | **成立（验签自动化）；端到端=人工，已承接** |
| **双语** | `desktop/src/i18n.ts`（`const zh` / `const en: Record<MessageKey, string>` / `TABLES` / `t()` / `useLocale`） | **跑过**：`desktop; pnpm test` → **12 文件 / 100 项全过**（含 `i18n.test.ts` 8 项、`i18n-literal-guard.test.ts` 3 项）；`node scripts/check-i18n.mjs` → **zh=160 en=160 条**，键集合 / 占位符一致、无空文案，exit 0；**独立深检**：`en` 段 166 行里**含中文的行 = 0**（没有「照抄中文当英文」） | **成立** |
| **双 ABI 发布** | `android/app/build.gradle.kts` 的 `splits.abi`（`include("arm64-v8a", "armeabi-v7a")`、`isUniversalApk = false`） | **本机复核通过**：现存产物 `app-arm64-v8a-release.apk` = 5463769 B = **5.21 MiB**、`app-armeabi-v7a-release.apk` = 4021489 B = **3.84 MiB**（与声明一致）；`apksigner verify --verbose`（build-tools 37.0.0）→ 两包 **Verifies** 且 **v2 scheme = true**。⚠️ 签名主体是 `CN=AudioLink Test`（`docs/42` §11 的表里也这么写）→ **是测试证书**，正式发布仍需真证书 | **成立**（数字与签名都本机复核过） |
| **绿色版** | `tools/tauri-portable.ps1`（产物 `target/evidence/release/portable/AudioLink_<ver>_x64_portable.zip` + `SHA256SUMS.txt`，`-Verify` 会解压并真启动一次） | **本机复核通过**：`AudioLink_0.1.0_x64_portable.zip` = 5502665 B；**重算 SHA256 与 `SHA256SUMS.txt` 逐位一致**（`A8A6E585…1F9E90`）；`-Verify` 真启动过 = 看板的人工读数（**无留存的运行日志**） | **成立**（哈希可复核；启动是一次性人工读数） |
| **用户文档** | `docs/manual/user-guide.zh-CN.md` / `user-guide.en-US.md` / `troubleshooting.zh-CN.md` / `troubleshooting.en-US.md` / `CONTRIBUTING.md` | 文件**成对存在**（7300/7613 B、5320/5578 B、5831 B）；README 已无「尚未实现」状态行（与正文声称一致） | **成立**（成对性 ✓；逐句对应未逐条比对 —— 见 §4） |
| **合规清单** | `docs/compliance/THIRD-PARTY-NOTICES.md`；桌面投放：`AboutPanel.tsx` → `ipc.thirdPartyNotices` → `EngineBridge::third_party_notices`（**先查 `resource_dir()` 再退回仓库路径**）+ `tauri.conf.json` 的 `bundle.resources` 映射；Android 投放：`android/app/src/main/assets/THIRD-PARTY-NOTICES.md` + `NoticesLoader` + `LicensesScreen`；`LICENSE` / `NOTICE` | 两个投放位文件**同尺寸 = 1056775 B**；`resources` 的源路径 → 运行时 `resource_dir()/THIRD-PARTY-NOTICES.md` **对得上**（`docs/42` §2.5 记过的坑已解）；`view::notices_tests::*`（2 项 PASS）守着「找不到时说清怎么生成」；`NoticesLoaderTest` 守 Android 侧；`git ls-files` **无任何密钥文件**、`keystore.properties` 未跟踪 | **成立**（唯一缺项是「隐私说明」，见 §2-C） |

### 附加核对

| 项 | 结果 |
|---|---|
| `App.tsx` 的过时注释 | **HEAD 版算错、工作区版算对**：`git show HEAD:desktop/src/App.tsx` 仍是「**不做**：托盘、开机自启、双语…（属 M5/M2）」；工作区（未提交）已自我更正。审计读到的是**工作区版**，这一点本人先写错过一次、已按 `git show HEAD:` 校正 |
| 窗口状态记忆（`docs/05` 交付物 1） | 实现 ✔（`tauri_plugin_window_state::Builder` + `tauri-plugin-window-state = "2.4"`）；**无自动化证据** |
| 发布流水线（交付物 3） | 看板有独立 Done 条目（release.yml 修复与首次真实运行、tag→Release、清单与产物同源校验）✔ |
| 剩余项承接 | 真机安装验收 → ~~面板 `[环境] MIUI 上 adb install release APK 失败 -99`（Todo）~~ **已按用户裁决移出跟踪（2026-09-18）**：MIUI/HyperOS 的系统侧限制，非本项目待办；代码签名证书 → `[M5] Windows 代码签名证书`（Todo）✔；真实更新流程验证 → `[M5] 真实更新流程验证`（Todo）✔ |
| 看板 M5 条目状态分布 | 20 条：**17 Done / 2 Todo / 1 In Progress（主行自身）** |

---

## 2. 挑出的错

### A. 正文漏记（4 项，性质：声明落后于实况）

`[M5]` 正文只写了双 ABI、双语、绿色版、用户文档；**托盘 / 桌面开机自启 / 自动更新（签名链路 + 安装包）/ 合规清单** 全部没有出现在正文里，而它们各自都有 Done 独立条目与代码落点。

### B. 活数字失真（3 处）

| 正文/条目里的数字 | 今天实测 | 判定 |
|---|---|---|
| 「139 条文案 × 2 语言」 | **160 条**（`check-i18n.mjs` 输出 `zh=160 en=160`；`zh` 段独立数也是 160） | **失真**（+15%） |
| `THIRD-PARTY-NOTICES.md`「1031.5 KB」 | **1056775 B = 1032.0 KiB** | **略失真**（差 519 B，说明后来重新生成过） |
| 便携 zip「5.37 MB」 | 5502665 B = **5.25 MiB**（5.37 = 脚本 `{Length/1KB}` 的 5373.7 **KB** 被写成 MB） | **口径错**（数值来源可复核，量纲写错） |
| （对照）「arm64-v8a 5.21 MB / armeabi-v7a 3.84 MB」 | 5463769 B = 5.21 MiB / 4021489 B = 3.84 MiB | **吻合** ✔（同一份声明里，MiB 口径的数字是准的） |

### C. 声明超前于实况（`docs/05` 交付物 vs 代码）

1. **M5 交付物 2 的「Android 开机自启」**：`AndroidManifest.xml` 与 `AudioLinkService` 的 KDoc 都写明**有意不做**（Android 15+ 禁止由 `BOOT_COMPLETED` 启动 mediaPlayback 前台服务，用户决策）→ **`docs/05` 未更新**，交付物清单仍在要求它。理由充分，是**文档没跟上决策**，不是实现缺失。
2. **M5 交付物 2 的「省电白名单引导」**：Android 侧全仓 grep `isIgnoringBatteryOptimizations` / `REQUEST_IGNORE_BATTERY_OPTIMIZATIONS` / `battery` → **零命中** = **未实现**；且看板上**没有任何条目认领它**。
3. **M5 交付物 5 的「隐私说明」**：全仓（278 个源码/文档文件）grep `隐私` / `privacy policy` / `不上传` → 只命中 `docs/manual/user-guide.zh-CN.md` 的**一句话**（「只在局域网内工作……不上传音频内容，不采集遥测到云端」）与 `docs/05` 自身的清单条目 → **没有独立材料**（无 `PRIVACY` 文件 / 无 docs 章节）。

### D. 文档未回溯（两处「未做表」与实况矛盾）

- `docs/41` §5「覆盖面」与 §6 仍写「**Android（Gradle）依赖仍未进清单**」；
- `docs/42` §2.5 仍写「Gradle 依赖还没进清单，Android 侧也没有声明的投放位置」；
- 而看板有两条 Done 条目（`[M5] Android 依赖进许可清单`、`[M5] Android 侧声明投放`），且我**实测** `android/app/src/main/assets/THIRD-PARTY-NOTICES.md` 与桌面板**同尺寸 1056775 B**、`NoticesLoader` + `LicensesScreen` 在。
→ **至少一处错**（文档滞后），需要统一口径。

### E. 声称完成但只有人工读数（无自动化守）

托盘、桌面自启、窗口状态记忆、`SettingsPanel`（含自启与自动连接两个开关，**无 `test.tsx`**）、绿色版 `-Verify` 真启动、真机安装 / 双 ABI 真机可用、真实更新端到端。**这些不是缺陷**（属 UI / 系统集成 / 真机范畴），但**面板不应让它们看起来像有自动化证据**。

---

## 3. 明判：能否判 Done

**按你给的标准（实现 + 自动化证据齐备、真机/外部项已由独立条目承接）：可以判 Done —— 但必须先补两条挂账，否则 Done 会掩盖两个未认领项。**

支持判 Done 的事实（我逐条跑过/复核过）：

1. **已认领的 8 项，实现全部在位**，且 6 项有可复核证据：双语（100 项 + 160/160 + en 零中文）、双 ABI（尺寸与 v2 签名本机复核）、绿色版（SHA256 逐位一致）、自动更新验签（12 项测试）、合规清单（双投放 + 无密钥）、用户文档（成对存在）；
2. **真机/外部项确实都有独立条目承接**：~~MIUI 安装（`[环境]` Todo）~~（**已按用户裁决移出跟踪 2026-09-18**：系统环境限制，非项目待办）、代码签名证书（Todo）、真实更新流程验证（Todo）；
3. **`App.tsx` 那条过时注释的修正在工作区、尚未提交**（HEAD 版仍过时）—— 判 Done 时按工作区状态记录即可，但别忘了它还没进历史。

判 Done 前**必须**先做的两件事（都不是实现缺口，是范围口径）：

1. **把「Android 省电白名单引导」明确处置**：要么挂一条 Todo（并说明它在 M5 验收 5 项里**没有**对应项，属交付物 2 的待办），要么把它移出 M5 范围并改 `docs/05`；现状是「零实现 + 无人认领 + 交付物清单仍在要求」——**这是本轮唯一真正会被 Done 掩盖的东西**。
2. **把「隐私说明」明确处置**：补一个独立落点（例如 `docs/privacy.md` 或在 `NOTICE`/用户手册里升格为正式条目），并在 `docs/05` 标注落点；现状只有用户手册里一句话，作为交付物 5 的一项**没有独立材料**。

同时建议（不阻塞 Done，但会被验收/发布现场咬到）：

- 正文重写：补记托盘 / 自启 / 自动更新 / 合规清单四项，并把「**真实更新流程验证**」列进剩余项（现正文漏）；
- 修 `docs/05` 交付物 2 的「Android 开机自启」表述（改为「有意不做 + 理由」）；
- 把「139 条」「1031.5 KB」「5.37 MB」三处活数字改成量级或删掉（口径统一为 MiB 或 KB，别混用）；
- 统一 `docs/41` §5/§6 与 `docs/42` §2.5 的 Android 依赖口径；
- 顺手补一两条「无自动化证据」项的最小护栏（例如 `SettingsPanel` 一条 vitest：自启开关读到 null 时不许显示成「关」）。

---

## 4. 跑了什么（实测记录）

```text
cargo nextest run -p audiolink-desktop --no-fail-fast
  → Starting 40 tests across 5 binaries (1 test skipped)
  → Summary: 40 tests run, 40 passed, 0 failed   [exit 0]
  （含 update::tests::* 4 · updater_signature 7 · updater_hosted_e2e 1 · view::notices_tests 2 ·
    settings::tests 4 · engine_seam 1；无 tray / autostart 用例 —— 故两项标记为「无自动化证据」）

cd desktop; pnpm test
  → Test Files 12 passed (12) · Tests 100 passed (100)   [exit 0]

cd desktop; node scripts/check-i18n.mjs
  → i18n：zh=160 en=160 条 · ✓ 键集合一致、占位符一致、无空文案   [exit 0]

# 独立深检（不依赖项目脚本）：en 段逐行扫 CJK
  → en 段 166 行 · 含中文的行数 = 0 · zh 段 160 条

# APK 尺寸 + 签名（本机复核，未重跑 assembleRelease）
  → app-arm64-v8a-release.apk    5463769 B = 5.21 MiB
  → app-armeabi-v7a-release.apk  4021489 B = 3.84 MiB
  → apksigner verify --verbose（build-tools 37.0.0）：两包 Verifies · v2 scheme = true

# 便携包校验和
  → AudioLink_0.1.0_x64_portable.zip 5502665 B
  → 重算 SHA256 与 SHA256SUMS.txt 逐位一致：True

# 合规材料与密钥
  → docs/compliance/THIRD-PARTY-NOTICES.md = 1056775 B（= Android assets 同名文件）
  → git ls-files 过滤 *.key/*.pem/*.p12/*.pfx/*.keystore/*.jks/secret → 空；keystore.properties 未跟踪
```

**没有跑**（并说明为什么）：

- `assembleRelease`/`pnpm tauri:build`：`android/keystore.properties` 未入库（正确设计，CI 注入），本机**无法**重新签名构建；且按纪律不与 8h soak 抢资源。
- 任何 `audiolink-engine` 的 `group_*` / `multi_session` 用例：soak 在跑，负载下会假红，本次与 M5 无关，**未跑**。
- `pnpm tauri:build --bundles nsis` 与真机安装：需要真机/长编译，属 M5 验收的真机项（已由 Todo 承接）。

---

## 5. 无法核实 / 证据强度分级

| 项 | 为什么无法核实 |
|---|---|
| 「无第三方私有 API 调用」（验收 5 的一部分） | 需人工审计全部依赖与调用点，无法机械判定；本次只复核了「无密钥」那一半 |
| 绿色版 `-Verify` 真启动过 | 只有看板的人工读数，没有留存的运行日志/截图 |
| 用户文档中英「逐句对应」 | 文件成对存在、章节结构同构，但未逐句比对（判定需人工或加一条结构对齐检查） |
| 托盘 / 桌面自启 / 窗口状态记忆的真实行为 | 需要桌面真机（Windows 会话）点击与重启验证 |
| Android 双 ABI 在 API 26 / API 34 真机可用 | 需真机（面板 Todo 正是这类） |

证据强度（沿用前两次审计的分级）：**确定性** = 代码 + 测试/可复核读数（本轮：双语、双 ABI 尺寸与签名、便携哈希、合规清单双投放、无密钥）｜**人工读数** = 只有看板记录（绿色版启动、托盘/自启行为）｜**无法核实** = 需真机或人工审计（真机安装、无私有 API）。

---

## 6. 一句话给 Lead

你的方向**对**：`[M5]` 正文确实漏记了 4 项（托盘 / 自启 / 自动更新 / 合规清单），而它们的实现与证据都在。
但「至此只剩真机安装验收」这句还漏了**真实更新流程验证**（面板有 Todo），更重要的是 `docs/05` 的交付物 2/5 里藏着**两个无人认领项**（Android 省电白名单引导、隐私说明）——
把它们处置掉（挂账或改范围）+ 修三处活数字口径，**我支持判 Done**。
