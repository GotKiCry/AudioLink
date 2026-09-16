# M5 · 发布链路盘点（从「能跑」到「能发给别人」）

> 日期 **2026-09-16**。这一份不是教程，是**现状盘点**：发布链路里哪些环节已经有了、
> 哪些还缺、缺的东西各自卡在哪。写它的理由很简单 —— 「产品化」最容易糊弄，
> 盘点写清楚，缺口就没法藏。

---

## 1. 已经有了什么

| 环节 | 现状 | 证据 |
|---|---|---|
| 版本单一来源 | ✅ | `tools/check-version.ps1` 校验 Cargo / tauri.conf / build.gradle 三处一致，CI 的 `version-consistency` job 每次都跑 |
| 桌面安装包配置 | ✅ 配置就绪 | `bundle.targets = ["nsis"]`，`nsis.languages = [SimpChinese, English]`，`webviewInstallMode = downloadBootstrapper` |
| 打包工具链 | ✅ 本地就绪 | `@tauri-apps/cli` 2.11 已装；NSIS 与 WixTools 缓存已在 `%LOCALAPPDATA%/tauri` |
| 桌面内核检查 | ✅ | CI `desktop` job：`pnpm build`（tsc 严格 + vite）+ `cargo check -p audiolink-desktop` |
| Android debug 包 | ✅ | CI `android` job：`cargo ndk`（arm64-v8a + armeabi-v7a）+ `assembleDebug` + artifact 上传 |
| Android release 签名 | ✅ 曾打通 | 签名 keystore + R8 规则（见 `docs/12` §8.5） |
| 合规材料 | ✅ | 许可审计 + 第三方声明 + 随包投放（`docs/41`） |

## 2. 还缺什么（按阻塞程度排）

### 2.1 自动更新的密钥是**占位符**

`tauri.conf.json` 的 `plugins.updater.pubkey` 现在是 `REPLACE_WITH_TAURI_UPDATER_PUBKEY`。
也就是说：**配置写了，功能不可用** —— 没有密钥对，签不出更新包，`createUpdaterArtifacts` 也是 `false`。

这一条最容易被"配置看起来都在"糊过去，所以单独列出来。

### 2.2 没有代码签名证书

Windows 上未签名的安装包会触发 SmartScreen 警告。证书是**外部资产**（要买、要实名），
不是代码能解决的问题 —— 只能诚实标注为「发布前需人工准备」。

### 2.3 没有发布流程（tag → 构建 → Release）

目前 CI 只在 push / PR 上跑检查，**没有任何 job 产出可分发的产物**（Android 的 debug APK 是构建验证，不是发布物）。
缺的是：tag 触发的 release job、产物命名规范、Release notes 模板、校验和。

### 2.4 Android 侧没有 release 打包流水线

release 签名曾人工打通，但 CI 里只有 `assembleDebug`。发布要的是 `assembleRelease` + keystore 注入（密钥走 CI secrets）。

### 2.5 Android 依赖的合规材料缺失

许可审计只覆盖 Rust 与前端（`docs/41` §5），Gradle 依赖还没进清单，Android 侧也没有声明的投放位置。

## 3. 打包的前置条件（这是有意的）

`tauri build` 会**因为找不到 `docs/compliance/THIRD-PARTY-NOTICES.md` 而失败** ——
因为 `bundle.resources` 收录了它。

```text
pwsh tools/license-audit.ps1 -Notices   # 先生成声明
pnpm tauri:build                        # 再打包
```

宁可打不出包，也不要打出一个没有声明的包。

---

## 4. 实测：首次真实打包（2026-09-16）

```text
pnpm tauri:build --bundles nsis
  → Finished `release` profile in 5m 14s（首次冷编译）
  → AudioLink_0.1.0_x64-setup.exe（3.69 MB）
```

安装包内容（`7z l` 直读安装包）：

| 条目 | 大小 |
|---|---|
| `audiolink-desktop.exe` | 15 089 152 B |
| `THIRD-PARTY-NOTICES.md` | 1 031.5 KB |

### 4.1 第一次打包就抓出一个真缺陷

`bundle.resources` 我一开始写成**数组**：

```json
"resources": ["../../docs/compliance/THIRD-PARTY-NOTICES.md"]
```

Tauri 把路径里的 `..` 转义成 `_up_`，于是安装包里是：

```text
_up_/_up_/docs/compliance/THIRD-PARTY-NOTICES.md
```

而运行时命令找的是 `resource_dir()/THIRD-PARTY-NOTICES.md` —— **两边对不上**，
装了包之后「关于」面板必然显示"未找到声明"。

改成 **map 形式**（源 → 目标）后：

```json
"resources": { "../../docs/compliance/THIRD-PARTY-NOTICES.md": "THIRD-PARTY-NOTICES.md" }
```

安装包内变成**根级**的 `THIRD-PARTY-NOTICES.md`，与运行时查找路径一致（重新打包 + `7z l` 复验）。

**为什么这件事值钱**：光看配置发现不了它 —— 只有真的打出包、真的去看包里的路径才会暴露。
这正是「真实安装包验证」的意义所在。

### 4.2 仍然没验的（诚实清单）

- **安装后实读**：本条验的是「资源进了安装包 + 包内路径与运行时查找一致」，
  **没有**在干净机器上安装后打开「关于」确认它真的读出来；
- Android 侧完全没有对应流程（§2.4、§2.5）；
- 安装包未签名（§2.2）。

---

## 5. 自动更新：从「配置在、功能不可用」到「签得出来」（2026-09-16 第二轮）

§2.1 记的那条（`pubkey` 是占位符、`createUpdaterArtifacts = false`）本轮动掉了：

| 步骤 | 结果 |
|---|---|
| 生成密钥对 | `pnpm tauri signer generate` → 私钥 `~/.tauri/audiolink.key`（**仓库外**），公钥写进 `tauri.conf.json` |
| 打开更新产物 | `createUpdaterArtifacts: true` |
| 签名打包 | `AudioLink_0.1.0_x64-setup.exe.sig`（0.4 KB，minisign 格式）实际产出 |
| 更新清单 | `tools/tauri-latest-json.ps1` → `latest.json`（版本 / URL / 签名） |

### 5.1 两个踩过的坑（都记下来）

**坑一：环境变量名。** 用 `TAURI_SIGNING_PRIVATE_KEY_PATH` 指向私钥文件时，Tauri 报
「A public key has been found, but no private key」—— 它要的是 `TAURI_SIGNING_PRIVATE_KEY`（私钥**内容**）。

**坑二：`.sig` 不是可选的。** 没有签名的更新包会被客户端直接拒绝，而 `tauri build` 在缺私钥时
**先产出安装包再报错** —— 于是很容易留下一个「看起来打好了、其实更新不可用」的包。
清单生成脚本把这一条钉死了：找不到 `.sig` 就抛错，并提示是不是漏了 `TAURI_SIGNING_PRIVATE_KEY`。

### 5.2 为什么单独写一个清单脚本

`tauri build` 只产出「安装包 + `.sig`」，而 updater 要的是一个 JSON 清单。缺这一步，
自动更新就永远停在「配置在、功能不可用」—— 这也正是 §2.1 那一条的实质。

清单里的 URL 指向 `releases/latest/download/<安装包名>`，与 `tauri.conf.json` 的 `updater.endpoints`
是同一个地址：**两处必须一致**，所以仓库名由脚本参数给出、不散落在各处。

### 5.3 仍未验的（诚实清单）

- **真实更新流程**：需要「旧版本 + 新版本 + Release 托管」三件东西同时在位 ——
  即 v0.1.0 装好，v0.1.1 发布，客户端检测到 `latest.json` → 下载 → 用公钥验签 → 安装。
  本轮做到的是「签得出来 + 清单能生成」，**不是**「装得上去」；
- **CI secrets 未配**：私钥要进 `TAURI_SIGNING_PRIVATE_KEY`（仓库设置，需人工操作）；
- **本地私钥是开发用的**：密码 `dev-only-change-me`，正式发布前应重新生成并妥善保管；
- 安装包仍未签名（§2.2 的代码签名证书是另一回事）。

---

## 6. 发布工作流：先审计，再相信（2026-09-16 第三轮）

`.github/workflows/release.yml` **从仓库骨架提交起就在那里，却从未真正跑过** —— 因为从没打过 tag。
「读一遍再信它」这件事本身就有价值：一读就发现四个问题。

| # | 问题 | 后果 |
|---|---|---|
| 1 | 产物路径写的是 `desktop/src-tauri/target/...` | 本仓库 `.cargo/config.toml` 把 target 固定成 `x86_64-pc-windows-msvc`，产物在**仓库根** `target/` —— 收集步骤必然空手而归，而 `-ErrorAction SilentlyContinue` 会把这件事**吞掉** |
| 2 | 没有生成第三方声明 | 桌面安装包要带 `THIRD-PARTY-NOTICES.md`（`bundle.resources` 收了它）→ 构建直接失败 |
| 3 | 没有生成 `latest.json` | 自动更新永远停在「配置在、功能不可用」（与 §5 同一个病） |
| 4 | 缺签名密钥时不给退路 | `createUpdaterArtifacts` 会因「有公钥没私钥」直接失败，而不是降级并说清楚 |

### 6.1 修正后的行为

| 场景 | 行为 |
|---|---|
| `workflow_dispatch`（默认 `publish=false`） | **只构建 + 上传 artifact，不发布** —— 用来验证发布链路本身 |
| `push v*.*.*` tag | 构建 → 创建 **草稿** Release（`draft: true`） |
| 有 `TAURI_SIGNING_PRIVATE_KEY` | 完整路径：签名 + 生成 `latest.json` |
| 没有签名密钥 | 降级构建（`--config` 临时关掉更新产物）+ `::warning::` 明确说「这份产物不能用于自动更新」 |
| 有 Android keystore secrets | `assembleRelease`（双 ABI） |
| 没有 | 降级 `assembleDebug` + `::warning::`「不是发布物」 |
| 产物收集为空 | **抛错**（不再是一张空表） |

**`draft: true` 是有意的**：发布是对外动作，留一道人工确认；而且草稿不进 `releases/latest`，
所以「草稿 → 手动发布」这一步同时承担了「对外可见」与「更新可见」两件事。

### 6.2 实测：首次真实运行（2026-09-16）

| 项 | 结果 |
|---|---|
| 触发 | `workflow_dispatch`（`publish=false`），run `35119443752` |
| `desktop installer` | **success**，580 s |
| `android apk` | success，94 s |
| `publish release` | **skipped**（`publish=false`，行为正确） |

第一次 dispatch 其实是**失败**的，两次修复都记在 §6 的表中（内联 JSON 被吞引号；以及我自己后来
把 `run: |` 行删掉，导致 YAML 结构破坏）。后者的表现值得记住：**GitHub 把 workflow 名字退化成文件路径、
每次 push 都触发它、`workflow_dispatch` 直接消失** —— 而本地 YAML 解析器**照样解析成功**。
所以「本地能解析」不等于「GitHub 能接受」，发布工作流必须真的跑一次才算数。

产物核对（`gh run download` 之后 `7z l` 直读安装包）：

| 条目 | 大小 |
|---|---|
| `AudioLink_0.1.0_x64-setup.exe` | 3 776.5 KB |
| ↳ `audiolink-desktop.exe` | 15 080 448 B |
| ↳ `THIRD-PARTY-NOTICES.md` | **根级**（与运行时 `resource_dir()` 查找一致） |

日志里那句 `::warning::未配置 TAURI_SIGNING_PRIVATE_KEY：本次产物不带更新签名（不能用于自动更新）`
正是设计要的效果：**没密钥也照跑，但绝不假装产物可用于自动更新**。

**仍未验**：`tag → 创建 Release` 这一段（要真的打 tag，那是对外动作）。




