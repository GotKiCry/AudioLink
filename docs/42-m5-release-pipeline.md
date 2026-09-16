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

