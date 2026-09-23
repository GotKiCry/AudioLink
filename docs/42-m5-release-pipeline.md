# M5 · 发布链路盘点（从「能跑」到「能发给别人」）

> ⚠️ **历史记录说明（2026-09-22 补）**：本文记录的是 **PIN 配对 / 信任库时代**的交付事实与证据。
> 该认证机制已在本轮整体移除（现在连上即用：没有配对码、没有白名单），本文中的配对步骤、PIN 验收数字与
> 信任判定均**不再对应当前实现**，只作为当时的交付记账保留 —— 现状见 `docs/72-remove-pairing.md`。

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
| Android debug 包 | ✅ | CI `android` job：`cargo ndk`（arm64-v8a + armeabi-v7a）+ `assembleDebug` + artifact 上传。2026-09-17 起分 ABI 产出两个：`app-arm64-v8a-debug.apk` / `app-armeabi-v7a-debug.apk`（见 §9） |
| Android release 签名 | ✅ 曾打通 | 签名 keystore + R8 规则（见 `docs/12` §8.5） |
| 合规材料 | ✅ | 许可审计 + 第三方声明 + 随包投放（`docs/41`） |

## 2. 还缺什么（按阻塞程度排）

> ⚠️ **本节是 2026-09-16 的缺口盘点，不是现状**。按时间顺序往下读：§2.1 由 §5 补、§2.3 由 §6/§8 补、
> §2.4 与 §2.5 由 §7/§9 与 §13 复核。**§2.2（代码签名证书）至今仍缺**。各条的原判断留在下文，不删。

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

（**本条已过时**：`release.yml` 的 android job 现在**有** `assembleRelease` 与 keystore secrets 注入（解出 `release.jks` + 写 `keystore.properties`），
缺 secrets 时明确降级 `assembleDebug` 并打 `::warning::`；本地 release 打包与双 ABI 也都真的跑过 —— 见 §6.1、§7、§9、§13。）

### 2.5 Android 依赖的合规材料缺失 ✅ 已补齐（见 §13）

许可审计只覆盖 Rust 与前端（`docs/41` §5），~~Gradle 依赖还没进清单，Android 侧也没有声明的投放位置。~~
→ **两项都已完成**：Gradle/Maven 依赖进了清单（`docs/41` §8，114 个组件），Android 侧也有随 APK 分发的声明与
应用内「开源许可」页（`docs/41` §9）。复核依据见 §13。

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
- ~~Android 侧完全没有对应流程（§2.4、§2.5）；~~ → **已补**：Android 侧现在有 release 打包路径（§7、§9）与随 APK 的声明投放（`docs/41` §9），见 §13；
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
| 有 Android keystore secrets | `assembleRelease`（分 ABI 两个包，见 §9） |
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

---

## 7. Android release 链路：本地把「签名 + R8」跑通（2026-09-16）

§2.4 那条「CI 里只有 `assembleDebug`」这一轮往前推了一步：**release 打包本身被真正验证过了**。

之前的问题是：CI 里没有 keystore secrets，所以那条 `assembleRelease` 路径永远是**降级**走的，
等于「配置写了但从没跑过」—— 和 release.yml 一开始的状态是同一类毛病。

于是本地用**测试 keystore** 把它跑了一遍：

```text
keytool -genkeypair ... -keystore android/release.jks      # 测试用，不入库（.gitignore 已含 *.jks）
（写 android/keystore.properties，同样不入库）
pwsh tools/gradlew.ps1 -JavaHome <JDK17> :app:assembleRelease
  → BUILD SUCCESSFUL in 1m 34s
  → app-release.apk  7.84 MB
```

| 核对项 | 结果 |
|---|---|
| 构建 | **BUILD SUCCESSFUL**，1m 34s |
| 体积 | release **7.84 MB** vs debug **19.17 MB** → **R8/minify 确实生效**（降 59%） |
| 签名 | `apksigner verify --print-certs` 通过：**V2 Signer**，证书 `CN=AudioLink Test, OU=Dev, O=AudioLink…` |
| 声明仍随包 | `assets\THIRD-PARTY-NOTICES.md`（1 056 775 B）在 **release** APK 里也在 |

**为什么 `7z l` 看不到 `META-INF/*.RSA` 是正常的**：现代 Android 用 **APK Signature Scheme v2**，
签名信息在 zip 结构里而不是 `META-INF`。以 `apksigner` 的输出为准，不以「有没有 RSA 文件」为准。

### 7.1 仍未验的

- **CI 侧的真 release 路径**：需要把 keystore 配进 secrets（人工操作，属仓库设置）；
- **装到真机上跑**：本轮验的是「能打出一个签名有效、R8 生效的 release APK」，不是「这个包装上能用」；
- 测试 keystore 只是本地验证用（密码就写在文档里），**不能用于发布**。

---

## 8. 发布链路跑到底：草稿 Release 真的建出来了（2026-09-16 第四轮）

§2.3 说的「没有发布流程」这一轮补齐并**跑到了最后一步**。过程中又发现一个自己的设计缺陷。

### 8.1 缺陷：手动 dispatch 时会把分支名当 tag

`release` job 的条件是 `tag 推送 || publish == true`，而创建 Release 用的 tag 取自 `GITHUB_REF_NAME`。
手动 dispatch 时那是**分支名**（`main`）—— 于是会创建一个叫 `main` 的 Release。

修法是让手动路径**必须显式给版本号**：加一个 `version` 输入，并在 job 里先把 tag 解析出来
（tag 触发用 tag 名，手动触发用 `v{version}`），解析不出来就 `::error::` 直接失败 ——
宁可这一步红，也不要建出一个名字错误的 Release。

### 8.2 实测（run `35126756341`，`workflow_dispatch` + `publish=true` + `version=0.1.1`）

| job | 结果 |
|---|---|
| `desktop installer` | **success**，612 s |
| `android apk` | success，112 s |
| `publish release` | **success**，12 s |

创建出来的草稿 Release：

| 核对项 | 结果 |
|---|---|
| Release | `v0.1.1`，**`draft = true`** |
| 资产 | `AudioLink_0.1.0_x64-setup.exe`（3803 KB）、`app-debug.apk`（19 770.2 KB） |
| **对外可见吗** | `GET /releases/latest` → **404 Not Found** —— 草稿确实不对外 |
| 打 tag 了吗 | **没有**（`git ls-remote --tags` 为空）—— 草稿发布不打 tag，这是 GitHub 的行为 |

验证完就把这个测试草稿删掉了（`gh release delete`），仓库回到没有 Release 的状态。

### 8.3 一个如实的观察

资产里的 APK 是 **`app-debug.apk`**：因为没配 keystore secrets，Android 走的是降级分支。
这正是设计要的效果（降级 + `::warning::`），但也说明：**要出真 release APK，还得配 secrets**（已立看板待办）。

### 8.4 剩下的

- **真发布**（把草稿点成正式）：一步人工动作，且要版本号真的对得上（`tools/check-version.ps1` 会保证三处一致）；
- 配好 secrets 之后，`latest.json` 才会随 Release 一起出去 —— 那才是自动更新真正可用的时刻。

---

## 10. 绿色版（便携包）：同一个 exe 的另一种形态（2026-09-17）

路线图 M5 交付物 3 要求发布「Windows 安装包 + 绿色版 + APK×2 ABI」。安装包与双 ABI APK 前两轮补齐了，
这一轮补绿色版：`tools/tauri-portable.ps1` 把桌面端 exe + 第三方声明 + 使用说明打成一个 zip。

| 项 | 值 |
|---|---|
| 产物 | `AudioLink_0.1.0_x64_portable.zip`，**5,373.7 KB**（exe 14,826 KB 压缩后） |
| 内含 | `AudioLink.exe` / `THIRD-PARTY-NOTICES.md`（1,032 KB）/ `README-portable.txt` |
| 校验和 | `target/evidence/release/portable/SHA256SUMS.txt` |

### 10.1 一个真实的正确性点：声明必须与 exe 同级

Tauri 的 `resource_dir()` 在**便携形态下就是 exe 所在目录**。安装版里 `bundle.resources` 会把
`THIRD-PARTY-NOTICES.md` 摆进资源目录，绿色版没人替它摆 —— 不摆的话「关于 / 第三方声明」面板会显示
「未找到声明文件」，而那是**合规项**，不是装饰。脚本把声明复制到与 exe 同级，并把理由写进脚本注释。

### 10.2 配置写用户目录（而不是 zip 里）

绿色版没有「配置随身携带」这个语义：`settings.json` / `identity/*.pem` 都在
`%APPDATA%\com.gotkicry.audiolink`。这与路线图 M5 的验收一致（「绿色版解压即用，配置写用户目录」），
README 里也写清楚了 —— 否则用户把目录拷去另一台机器，会以为设置「丢了」（其实是设计如此）。

### 10.3 验证

| 项 | 结果 |
|---|---|
| 打 zip | `pwsh tools/tauri-portable.ps1` → 5,373.7 KB + SHA256 |
| zip 内容 | 三个条目，用 zip 目录逐条核对（名称与体积都对得上） |
| **解压即用** | `-Verify`：解压到临时目录 → 启动 exe → **进程存活 8 s**（不是「应该能启动」，是真启动过） |
| CI | `release.yml` 的 desktop job 加了打包步骤；产物随 artifact 进 Release（`files: dist/**/*` 自动带上） |

### 10.4 未验 / 说明

- 「启动后写用户目录」这一次**没有**观察到 `window-state.json` 的时间戳变化 —— 因为 `-Verify` 用
  `Stop-Process -Force` 结束进程，而窗口状态是**正常退出**时才写的；AudioLink 又是「关窗不退出」
  （托盘常驻）的设计，无法在不做 GUI 交互的前提下优雅退出。这一条按**代码事实**陈述：
  配置路径来自 `app_config_dir()`，与安装形态无关（现有的 `identity/` 就在那里；旧版本留下的 `trust.json` 当前版本不读不写）；
- README 里写的 WebView2 依赖只做了说明，没有在**缺 WebView2 的机器**上实测（本机有，这条属真机环境）；
- 绿色版**没有**做代码签名：与安装包同一条待办（`[M5] Windows 代码签名证书`）。

---

## 9. 分 ABI 打包：把「双 ABI」变成「两个包」（2026-09-17）

此前 `assembleRelease` 产出**一个** `app-release.apk`（7.84 MB），里面同时装着 arm64-v8a 与 armeabi-v7a
两份 `libaudiolink_ffi.so` 加两份 `libjnidispatch.so` —— 也就是每台设备都下载一份永远用不到的本地库。
路线图 M5 交付物 3 写的是「APK×2 ABI」，本轮把它落成 `splits.abi`：

```kotlin
splits {
    abi {
        isEnable = true
        reset()
        include("arm64-v8a", "armeabi-v7a") // FR-40 的契约，也是本文件的唯一声明处
        isUniversalApk = false
    }
}
```

| 产物 | 体积 | 内含 `lib/` | 签名 |
|---|---|---|---|
| `app-arm64-v8a-release.apk` | **5.21 MB**（原 7.84 MB，−33.5%） | arm64-v8a × 4 | V2 Signer `CN=AudioLink Test` |
| `app-armeabi-v7a-release.apk` | **3.84 MB**（−51.0%） | armeabi-v7a × 4 | V2 Signer `CN=AudioLink Test` |

两个包的 `lib/` 用 zip 目录逐条核对过：arm64 包里只有 `lib/arm64-v8a/*`，v7a 包里只有 `lib/armeabi-v7a/*`。
debug 变体同样分 ABI（16.54 / 15.17 MB，原 universal debug 19.17 MB）。

### 9.1 踩到的真缺陷：ABI 不能两处声明

第一版把 ABI 列表同时写在 `defaultConfig.ndk.abiFilters` 与 `splits.abi.include`，AGP 直接拒绝：

```text
Conflicting configuration : 'armeabi-v7a,arm64-v8a' in ndk abiFilters
cannot be present when splits abi filters are set : armeabi-v7a,arm64-v8a
```

修法是把 `ndk { abiFilters }` 整块删掉，ABI 集合只在 `splits.abi.include` 声明**一处**。
两处各写一份的下场是「改了一处、另一处悄悄还在」；这次 AGP 把它变成硬错误，算是走运。

### 9.2 顺带补的护栏：清理不再构建的 ABI 产物

`build-rust.ps1` 过去只往 `jniLibs/<abi>/` 写，从不清理。ABI 集合一变，旧 ABI 的 `.so` 就留在目录里**骗人**：
打包时会把它挡在包外，但目录看起来仍像「这个 ABI 还在支持」。现在脚本会删掉不属于本次 `-Abi` 的目录。

### 9.3 仍未验的（诚实清单）

- 两个分 ABI 的包**装到真机**跑一遍（arm64 真机 / 32 位真机）—— 与 M1/M2 真机验收是同一条阻塞；
- CI 侧的产物搬运是通用的（`find android/app/build/outputs/apk -name "*.apk"`），**不需要改**；
  下次 tag 发布会看到两个 APK 一起进 Release 资产。

### 9.4 一个必须写下来的判断：FR-40 的 ABI 集合没有被动

本轮最初把「双 ABI」理解成 arm64-v8a + x86_64（真机 + 模拟器），改完才发现 `docs/01` 的 **FR-40 写的是
arm64-v8a + armeabi-v7a**。已全部回退 —— 需求文档是契约，支持面既不该凭「现代设备都是 arm64」收窄，
也不该凭「模拟器方便」扩张。要改这个集合，先改 FR-40 并写清理由，再动代码。

---

## 11. 更新清单与产物同源校验（2026-09-17）

自动更新链路是「读 `latest.json` → 下载 → **验签** → 安装」。前两步肉眼可查，第三步不行 ——
签名对不对只有算一遍才知道。

新增 `tools/check-update.mjs`：读 `latest.json` 与 `tauri.conf.json` 的公钥，对本地安装包做一次**真正的 Ed25519 验签**
（minisign 的 prehashed 模式，签名对象是 `BLAKE2b-512(file)`；工具两种模式都试并把命中的那种报出来）。

**为什么用 Node 而不是 PowerShell**：Ed25519 与 BLAKE2b-512 都是 Node 内置（`crypto`），而 PowerShell 的 .NET
没有 BLAKE2b、也不保证有 Ed25519 —— 自己实现一遍密码学不是这里该做的事。

### 11.1 第一次运行就抓到一个真缺陷

工具跑起来第一件事是**报错**：`latest.json` 里的签名（420 字符 base64）与 `.sig` 文件（560 字符）**不是同一份**。
时间戳对得上：清单 `pub_date` 是 `09-16T15:40`，而安装包与 `.sig` 是 `09-17 00:45` —— **清单比产物旧了 9 小时**。

含义：`latest.json` 当初是从 `.sig` 生成的，但之后**重新构建过安装包**（`.sig` 随之变化）而没人重新生成清单。
客户端拿这份清单去下载新包 → **验签必然失败** → 用户看到的是一句和「签名不匹配」有关的错误，
而真正的原因（清单过期）藏在别处。

已修：用 `tools/tauri-latest-json.ps1` 从当前 `.sig` 重新生成 → **验签通过**。

### 11.2 验证（含破坏性）

| 场景 | 结果 |
|---|---|
| 清单与当前 `.sig` 同源 | **✓ 通过** `[prehashed (BLAKE2b-512)]` |
| 把安装包**改一个字节** | **✗ 失败**（exit 1）—— 证明它真的在验密码学，不是只比字符串 |
| 用过期副本（`target/evidence/release/artifacts/`） | **✗ 失败** —— 能区分「清单与这个文件是否配套」 |

### 11.3 接进发布流程

`release.yml` 在生成 `latest.json` 之后跑一次校验（只在有清单时跑，降级构建不会被误伤）——
**清单过期会让发布流程当场失败**，而不是等用户更新失败才发现。

### 11.4 仍未做

真正的「装上去」：需要两个版本 + Release 托管 + 干净机器（看板的 `[M5] 真实更新流程验证`）。
本轮做完的是**发布前能自动核对的那一半**。

---

## 12. 自动更新端到端验证：一条命令 + 一条人工清单（2026-09-17）

§5.3 与 §11.4 留着同一句话没兑现：「真的装上去」需要**两个版本 + Release 托管 + 干净机器**。
这一轮把它拆开：**能自动化的全部自动化，剩下的收敛成一条能照做的人工清单**。

### 12.1 一条命令

    pwsh -File tools/updater-local-e2e.ps1

六个阶段，每段独立判定 PASS/FAIL（一段失败不会遮住后面的阶段）：

| 阶段 | 断言 | 2026-09-17 实测（真实产物 3,793.5 KB） |
|---|---|---|
| S1 | 由 `tools/tauri-latest-json.ps1` 从当前 `.sig` 生成 `latest.json` | `version=0.1.0`；签名前 40 字符 `dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBm…` |
| S2 | 清单 + 安装包 + `.sig` 组装成一个可托管目录 | `serve/` 下三个文件 |
| S3 | 在 127.0.0.1 起静态托管（HttpListener；不联网、不依赖 python） | `http://127.0.0.1:8321` 就绪 |
| S4 | 走 HTTP 取清单与安装包：可达 + 字段齐全 + 字节一致 | `sha256=E5A99CA19A7773C9…`（HTTP 取回 == 托管文件） |
| S5 | 版本比较语义：新 / 同 / 旧 | `0.1.0 vs 0.0.9 → 1`、`vs 0.1.0 → 0`、`vs 0.9.9 → -1` |
| S6 | 用**配置公钥**验签**从 HTTP 取回**的字节；再改 1 字节必须被拒 | `E2E-HOSTED: positive ok bytes=3884530` / `E2E-HOSTED: negative rejected InvalidSignature` |
| S7 | **真插件**驱动：本地清单当更新源，`check()` + `download()` 真验签（含篡改/错公钥负例） | `PROBE: plugin end-to-end ok` / `Minisign(InvalidSignature)` / `Minisign(UnexpectedKeyId)` |

退出码就是门禁语义，刻意区分「真的验证了」与「只是没报错」：

| 退出码 | 含义 | 实测 |
|---|---|---|
| 0 | 六段全部通过（含验签正例与负例） | ✓ 见上表 |
| 1 | 有断言失败 | ✓ `-TamperHostedPackage` 场景（§12.2） |
| 2 | **缺少带签名的构建产物 → 验签环节根本没执行**（不等于通过） | ✓ `-BundleDir` 指向空目录 |

### 12.2 它怎么证明自己不是「只会打印 PASS」

上面那两个非 0 退出码就是证据本身（2026-09-17 真跑）：

（1）**破坏性自检**：组装完成后把托管包改 1 字节（清单里的签名一个字符都不动）：

    pwsh -File tools/updater-local-e2e.ps1 -TamperHostedPackage
    # 退出码 1
    FAIL  S6 验签：配置公钥 + 从 HTTP 取回的字节（正例通过 / 改 1 字节被拒）
          Rust 端到端验签测试失败（exit 101）
    thread 'hosted_release_end_to_end_from_local_http' panicked at desktop\src-tauri\tests\updater_hosted_e2e.rs:179:10:
    本地托管的发布包 + 清单签名 + 配置公钥必须验签通过: InvalidSignature

（2）**缺产物**：`-BundleDir` 指到一个空目录 → 退出码 2，并打印
「缺少「带签名的安装包」：验签环节无法执行 —— 本脚本不假装通过」，同时告诉你产物怎么造。

脚本还额外防了一件事：S6 跑完会检查 Rust 输出里**有没有那两条标记行**（`E2E-HOSTED: positive ok` /
`E2E-HOSTED: negative rejected InvalidSignature`）——少任何一条就判失败，
避免「测试被跳过」被当成「验证通过」。

### 12.3 边界：哪一环由谁覆盖

| 环节 | 覆盖者 | 能进 CI 吗 |
|---|---|---|
| 清单生成 | `tools/tauri-latest-json.ps1`（S1） | ✓ 已在 `release.yml` |
| 清单 ↔ 本地产物配套（离线验签） | `tools/check-update.mjs` | ✓ 已在 `release.yml` |
| 篡改包被拒（纯算法） | `tests/updater_signature.rs`（8 个用例） | ✓ 每次 `cargo test` |
| 篡改包被拒（**从 HTTP 取回的真实字节**） | `tests/updater_hosted_e2e.rs` + 本脚手架（S4/S6） | ⚠ 需要带签名的产物：release job 可以，PR CI 只会拿到退出码 2 |
| 本地托管 + HTTP 可达 + 字段 + 版本语义 | 本脚手架（S3/S4/S5） | ⚠ 同上 |
| 插件**真的**按 `endpoints` 拉清单与包 | `tools/updater-plugin-probe`（S7） | ✓ **debug 语义**下已自动；release 构建**拒绝 http 端点**（§12.6） |
| 下载后的**真安装**（干净机器） | 无 | ✗ 见 §12.5 |

想接进发布流程（本轮**没有改 CI**，改 `.github/workflows/release.yml` 是另一件事）：
在「Verify update manifest against the artifacts」之后加一步 `- run: pwsh -File tools/updater-local-e2e.ps1`。
release job 有签名产物，六段会真的跑完；**PR CI 不要加** —— 那里没有 `TAURI_SIGNING_PRIVATE_KEY`，只会拿到退出码 2。

### 12.4 必须人工的四步（为什么 + 怎么做）

**第 1 步：把版本提到 0.1.1（三处；单一来源是根 Cargo.toml）**

| 文件 | 字段 | 改成 |
|---|---|---|
| `Cargo.toml` | `[workspace.package] version` | `0.1.1` |
| `desktop/src-tauri/tauri.conf.json` | `version` | `0.1.1` |
| `android/app/build.gradle.kts` | `versionName` | `0.1.1` |

然后跑 `pwsh -File tools/check-version.ps1` → 期望最后一行是 **`版本一致：0.1.1`**
（不一致它会 `exit 1`，并把哪一份对不上列出来）。

为什么人工：版本号是发布事实；改漏一处，客户端会**永远**认为「已是最新」。

**第 2 步：发布（草稿 → 正式）**

- 触发（二选一）：`gh workflow run release.yml -f publish=true -f version=0.1.1`，
  或者 `git tag v0.1.1; git push origin v0.1.1`。
- 观察 1：`desktop installer` job 里「Generate update manifest」与
  「Verify update manifest against the artifacts」都是 **success**。后者就是 `tools/check-update.mjs` ——
  清单与产物不配套会**当场红**（§11.1 抓过这种）。
- 观察 2：草稿 Release 的资产里应有 `AudioLink_0.1.1_x64-setup.exe`、它的 `.sig`、以及 `latest.json`。
- ⚠ **坑（§8.2 实测过）**：手动 dispatch 的 `version` **只决定 tag 名**，资产名与清单里的版本跟着
  **代码里的版本号**走。所以必须先做第 1 步，否则会得到「tag 是 v0.1.1、资产却叫 `AudioLink_0.1.0_…`」的错位组合。
- ⚠ **草稿不对外**：`endpoints` 指向 `releases/latest/download/latest.json`，而 `releases/latest`
  **只认正式 Release**（§8.2 实测：草稿状态下该地址是 404）。必须把草稿点成 **Publish release**。

为什么人工：对外发布不可逆，而且需要仓库写权限。

**第 3 步：干净机器装 v0.1.0**

- 在**另一台机器**（或同一台机器的另一个 Windows 用户）上装 `AudioLink_0.1.0_x64-setup.exe`。
- 观察：关于/托盘显示的版本是 **0.1.0**。
- 为什么人工：安装会写 `Program Files` / 注册表，「干净」本身是环境属性。

**第 4 步：客户端检测 → 下载 → 验签 → 安装**

- 打开「软件更新」面板 → 点 **检查更新**。
- 观察 A（验「检测」）：面板显示 **发现新版本 0.1.1（当前 0.1.0）**，带发布日期与更新说明。
  若显示的是错误：把界面上的**中文错误原文**贴回来 —— 错误层已翻成人话（例如与签名相关的失败），
  那通常意味着 `latest.json` 与安装包不配套（§11.1 的病）。
- 点 **下载并安装 0.1.1** → 出现确认提示（「安装会先与对端优雅收尾……现在安装吗？」）→ 点 **确认安装**。
- 观察 B（验「下载 + 验签」）：状态显示 **下载中…（验签通过后才会启动安装器）**，
  随后 AudioLink 退出、安装器接管、装完自动重开。
- 观察 C（验「装上去」）：重开后版本是 **0.1.1**；再点一次「检查更新」显示 **已是最新版本（0.1.1）**。
- 为什么人工：需要真实网络到 GitHub + 真安装器进程；而且 `endpoints` 是**编译期**常量，
  脚手架无法让**已经编译好的**客户端改去访问本地托管。

**（可选）只想验「下载 + 验签」而不动 Release**

临时把 `desktop/src-tauri/tauri.conf.json` 的 `plugins.updater.endpoints` 改成
`["http://127.0.0.1:8321/latest.json"]`，再把托管清单里 `platforms.windows-x86_64.url` 改成本地地址
（脚手架生成的清单 url 指向 GitHub），然后 `pnpm tauri:dev` 里点「检查更新」。

走这条路时**下载与验签是真的**（同一个插件、同一把公钥），但**安装器仍会真的运行** ——
不要在正在用的机器上点安装；验证完记得把 `endpoints` 改回去。

### 12.5 仍未做（诚实清单）

- ~~插件真的按 `endpoints` 拉取没有自动化~~ → **§12.6 已补上**（独立 crate `tools/updater-plugin-probe`，debug 语义下真拉、真下载、真验签）。
  仍未做的：正式（release）语义下把本地站点当更新源 —— 插件**拒绝** http 端点（§12.6 实测），生产只能 https。
- **真安装**没有自动化（改系统状态；且 Windows 上会 `exit(0)` 拉起安装器）。
- **CI 未接线**：只写了怎么接（§12.3 末段），改 `.github/workflows/release.yml` 不在本轮 write scope。
- 版本比较用的是与插件一致的判据（`remote > current`）+ 简化的 x.y.z 比较：**pre-release 语义**
  （如 `0.1.1-beta`）没有覆盖；需要时以插件里的 `semver` crate 为准。

### 12.6 真插件驱动（S7）：把「驱动入口」这个卡点补掉（2026-09-17 补）

S1–S6 全是本机的，但它们**没有驱动真插件** —— 验签用的是 tauri-plugin-updater 调用序列的复刻。
真插件的入口（`UpdaterExt::updater()` / `Update::download()`）需要 `AppHandle`
（`UpdaterExt` 实现在 `T: Manager<R>` 上），集成测试里构造它要给 tauri 开 `test` feature ——
而 `desktop/src-tauri/Cargo.toml` 是**产品**依赖清单，不该为测试塞 test-only feature。

于是新增独立 crate **`tools/updater-plugin-probe`**：自带 `[workspace]`、**不是** AudioLink workspace 成员，
所以它的 test-only 依赖不进产品依赖图：

    tauri = { version = "2.11", default-features = false, features = ["test"] }
    tauri-plugin-updater = "2.11"

它用 tauri 的 mock runtime 建 AppHandle，往 app config 里**注入**插件配置（endpoints 指本地 127.0.0.1、
pubkey 用产品 `tauri.conf.json` 里那一把），然后真调 `check()` 与 `download()`。
改的是**进程内的 mock config**，不碰仓库里的 `tauri.conf.json`。

实测（2026-09-17，`pwsh -File tools/updater-local-e2e.ps1` 的 S7 阶段，整体 exit 0）：

| 场景 | 真插件的输出 |
|---|---|
| 正例：本地清单 version=0.1.1（客户端 0.1.0）+ 真包 | `PROBE: check ok remote_version=0.1.1` / `PROBE: download ok bytes=3884530` / `PROBE: plugin end-to-end ok` |
| 负例：托管包改 1 字节（清单签名一字未动） | `PROBE: download rejected as expected: Minisign(InvalidSignature)` |
| 负例：pubkey 换成另一把 | `PROBE: download rejected as expected: Minisign(UnexpectedKeyId)` |

失败原因能直接读出来：**InvalidSignature = 签名与内容对不上**；**UnexpectedKeyId = 签名不是这把公钥签的**。

#### 一个必须写下来的事实：http 端点只在 debug 下能用

不打开 `dangerousInsecureTransportProtocol` 时，插件的 endpoint 校验按构建 profile 表现**不同**
（`tauri-plugin-updater` 的 `config.rs`：那个 `return Err(InsecureTransportProtocol)` 在
`#[cfg(not(debug_assertions))]` 里）：

| 构建 | http:// 端点的处理 | 实测 |
|---|---|---|
| debug（dev） | **只警告，仍然接受** | `[WARNING] The updater endpoint "http://127.0.0.1:8321/latest.json" doesn't use https protocol...` |
| release（正式包） | **直接拒绝** | `failed to initialize plugin updater: ... The configured updater endpoint must use a secure protocol like https.` |

这条直接决定「本地托管能不能替代 Release 托管」：

* 本机自动化**能在 debug 语义下**把「插件真拉 + 真验签」验到底 ✓；
* 但**正式构建必须走 https** ⇒ 生产路径只能是 GitHub Release，本地 http 托管**不可能**替代它。
  `dangerousInsecureTransportProtocol` 因此**绝不能进产品配置**（它是危险开关）。

release 反证怎么复跑（一次性，会编 release，几分钟）：

    $env:CARGO_TARGET_DIR = 'target'
    cargo run --release --manifest-path tools/updater-plugin-probe/Cargo.toml -- --base http://127.0.0.1:8321 --expect-insecure-rejected --require-reject

### 12.7 三分表：这条 Todo 的 Done 依据（由事实决定）

**① 本机已能自动验（可重复，有 exit code）**

| 环节 | 命令 | 结果 |
|---|---|---|
| 清单生成 + 托管 + 字段 + 字节一致 + 版本语义 + 复刻验签 | `pwsh -File tools/updater-local-e2e.ps1` | **exit 0**，S1–S6 全 PASS |
| **真插件** check() + download() 真验签（正例 / 篡改 / 错公钥） | 同上（S7） | **exit 0**，probe 标记行齐 |
| 纯算法验签 + 4 类篡改拒绝 + 真产物验签 | `cargo test -p audiolink-desktop` | exit 0（updater_signature 8 项） |
| 清单 ↔ 本地产物离线自洽 | `node tools/check-update.mjs` | 已在 `release.yml` |
| 正式构建拒绝 http 端点（反证） | §12.6 末的 release 命令 | `PROBE: insecure endpoint rejected as expected` |

**② 仍属人工（本机做得到，但要人操作/看界面）**

| 环节 | 为什么人工 | 照做步骤 |
|---|---|---|
| dev 里点「检查更新」看到 0.1.1、再点安装 | 要起 GUI 并点按钮（用 `--config` 把 endpoints 覆盖到本地托管） | §12.4 的可选段 |
| 真安装（拉起安装器、`exit(0)`、装完自动重开） | 会改系统状态 | §12.4 第 4 步 |

**③ 必须外部（不可本地化）**

| 环节 | 为什么 |
|---|---|
| GitHub Release 正式发布（草稿 → Publish） | 对外不可逆动作 + 需要仓库写权限；`releases/latest` 只认正式 Release |
| 生产语义下的 Release 托管（https） | 正式构建**拒绝 http 端点**（§12.6 实测）⇒ 只能真 https（GitHub） |
| 干净机器装 v0.1.0 → 升到 0.1.1 | 改 `Program Files` / 注册表；「干净」是环境属性 |

**结论（依据上面三张表）**：这条 Todo **不能整体收口**。
「验签」这一件已经从算法到**真插件**全部自动化；「Release 托管」本机只能在 debug 语义下验证，
生产语义必须真 https；「两版本」只有版本比较语义与清单版本差（0.1.1 vs 0.1.0）可自动，
**真升级**必须装到干净机器。剩下那三个外部动作，建议单开条目承接。

---

## 13. 第 110 轮口径复核（2026-09-17）

审计（`docs/53-m5-audit.md` §2-D）点名：本文 §2.5 与 `docs/41` §5/§6 仍写「Android 依赖未进清单 /
没有声明的投放位置」，而实况是两项都已做。逐句复核结论如下（`docs/41` 侧的同批改动记在 `docs/41` §10）。

| 原表述 | 判定 | 今天实况 | 依据（符号名 / 可复核读数） |
|---|---|---|---|
| §2.5「许可审计只覆盖 Rust 与前端，Gradle 依赖还没进清单，Android 侧也没有声明的投放位置」 | **过时** | ① Gradle/Maven 依赖**进了清单**：114 组件（113 allowed / 1 notice / 0 denied）；② Android 投放**已在**：`assets/THIRD-PARTY-NOTICES.md` + `NoticesLoader` + `LicensesScreen`，入口是主界面顶栏「开源许可」；③ 桌面与 Android 两个投放位文件**同尺寸 1 056 775 B** | `tools/android-licenses.ps1`（采集）→ `tools/license-audit.ps1`（判定 + 同步 assets）；`docs/compliance/license-report.md` 的 Android 节；`android/app/src/main/assets/THIRD-PARTY-NOTICES.md` |
| §2.4「CI 里只有 `assembleDebug`」 | **过时** | `release.yml` 的 android job 有 `assembleRelease`（四件 keystore secrets → 解出 `release.jks` + 写 `keystore.properties`）；缺 secrets 时**明确降级** `assembleDebug` 并打 `::warning::未配置 Android keystore：本次只出 debug APK（不是发布物）` | `.github/workflows/release.yml` 的 android job |
| §4.2「Android 侧完全没有对应流程」 | **过时** | 同上两条（打包路径 + 声明投放都在） | —— |
| §2.1「pubkey 是占位符 / `createUpdaterArtifacts=false`」 | **已由 §5 注明补掉**（无需再改） | 密钥对生成、`createUpdaterArtifacts: true`、`.sig` 实际产出 | §5 |
| §2.3「没有发布流程」 | **已由 §6/§8 注明补掉** | `release.yml` 跑通过、草稿 Release 真建出来过 | §6、§8 |
| §2.2「没有代码签名证书」 | **仍成立** | 安装包与绿色版都未签名 | §10.4；看板 `[M5] Windows 代码签名证书`（Todo）|
| §12.5「CI 未接线（`updater-local-e2e.ps1`）」 | **仍成立** | `release.yml` / `ci.yml` 里都没有这一步（本轮 grep 复核） | `.github/workflows/release.yml`、`ci.yml` |
| §12.5「真安装没有自动化」「插件按 `endpoints` 拉取没有自动化」 | **仍成立** | 见 §12.3 的覆盖表 | §12.3 |
| §9.3「两个分 ABI 的包装到真机」 | **仍成立** | 与真机验收同一条阻塞 | §9.3 |

### 13.1 顺带发现（**范围外**，未改，需另行处置）

1. **CI 生成的声明可能不含 Android 一栏**：`ci.yml` 的 `License audit (Rust + frontend)` 与 `release.yml` 的
   `Generate third-party notices` 都**不先跑** `tools/android-licenses.ps1`；脚本找不到
   `target/evidence/compliance/android-licenses.json` 时只打印「未找到 Android 依赖清单，本次跳过 Android」。
   本仓库里那份 1 056 775 B 的声明是**本机手动跑过采集**才含 Android 组件的 —— 换言之，发布产物若在 CI 上生成，
   Android 一栏会**静默缺席**（有提示但不失败）。建议二选一：把采集接进两处 workflow，或让 `-Notices` 在缺 Android 清单时**明确失败**。
2. `ci.yml` 那一步的名字仍叫 `License audit (Rust + frontend)`，与脚本现在的实际覆盖面（默认含 Android）不符。
3. `docs/compliance/license-report.md` 里「未覆盖：…Android 侧的**投放位置**（声明入口）」是**自动生成的**，
   模板在 `core/crates/audiolink-tools/src/license.rs` 的报告渲染函数里 —— 见 `docs/41` §10.1。
---

## 14. 发布链路收口：正式密钥 + secrets + 一个会让 CI 必红的缺口（2026-09-20）

§13 的缺口清单本轮全部落地或明确边界。四件事，按重要性排。

### 14.1 密钥换代（趁「从未发布过」的零成本窗口）

**为什么此刻能动**：`git tag -l` 与远端 tag 都是空的 —— 从没有正式发布过，换 key 不影响任何已装用户。
**一旦发过版，这笔账就永远关上了**（Tauri 换 key = 老用户收不到更新；Android 换 keystore = 无法覆盖安装）。

| | 旧（本轮替换掉） | 新（现役） |
|---|---|---|
| Tauri 更新签名 | `~/.tauri/audiolink.key`，密码 `dev-only-change-me`（§5.3 自注「开发用，发布前应重新生成」） | 同路径，24 字节随机口令；**公钥已写进 `tauri.conf.json`** |
| Android keystore | `android/release.jks`，`storePassword=test-password`、`CN=AudioLink Test` | `~/.audiolink/audiolink-release.jks`，随机口令、`CN=AudioLink Release`、RSA 4096 / 10000 天 |

- **证书指纹（Android）**：`59:3B:77:66:DB:09:56:FE:07:3C:25:1D:F3:6C:81:91:5A:25:3A:6A:7C:08:17:3A:40:E6:A3:6F:EA:34:9C:3C`；
- **口令与清单**落在**仓库外** `~/.audiolink/`（`secrets.env` + `README.md`）；旧资产改名留档（`audiolink.key.dev-backup-20260920`）；
- ⚠️ **备份是硬要求**：这两个文件丢了，就再也签不出「能被老版本接受」的更新包。

### 14.2 CI secrets 已配（六条）

`gh secret list` 实测在位：`TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` /
`ANDROID_KEYSTORE_BASE64` / `ANDROID_STORE_PASSWORD` / `ANDROID_KEY_ALIAS` / `ANDROID_KEY_PASSWORD`。
于是 `release.yml` 里那两条 `::warning::`（「本次产物不带更新签名」/「只出 debug APK」）**不会再出现**。

### 14.3 一个会让 CI 必红的缺口（本轮修掉）

**事实**：`0bf3f56`（2026-09-17 17:24）把 `license-audit.ps1 -Notices` 改成「缺 Android 清单 → **明确失败**」
（这是对的：声明随包发给用户，少一栏比流水线红更坏）。但 `release.yml` 的 desktop job 里那一步**从来没有**
Android 清单 —— 它跑在 `windows-latest`，而 Android 依赖只有 android job（有 Gradle）采得到。

⇒ **从 09-17 17:24 起，release 流水线必红**（desktop job 在 `Generate third-party notices` 就抛错）。
最后一次 release 运行是 09-16 17:12（成功）—— 之后一次都没跑过，所以没人发现。

**修法**：

| 位置 | 改动 |
|---|---|
| `release.yml` android job | 新增「依赖树（releaseRuntimeClasspath）」+「采集许可」两步，把 `android-licenses.json` 上传为 artifact |
| `release.yml` desktop job | `needs: [android]` + `download-artifact` 到 `target/evidence/compliance/` |
| `ci.yml` android job | 同样两步（只验「采得到」，防采集脚本悄悄腐烂） |

⚠️ CI 的 Gradle 缓存在 `~/.gradle/caches/modules-2/...`，本地在 `.gradle-home/...` —— 采集步骤显式传 `-Cache`。

### 14.4 自动更新端到端接进发布流程

`tools/updater-local-e2e.ps1`（§12.6 的 S1–S7）此前**只在本地跑**（§13 把它列为缺口）。现在进了
`release.yml` 的 desktop job，条件与同源校验一致（有 `latest.json` 才跑）。它在这里的价值最高：
私钥配错、公钥没跟着换，只有这一步能当场抓住 —— 否则产出的是一批「装得上、永远更新不了」的包。

### 14.5 本轮实测证据

| 验证 | 命令 | 结果 |
|---|---|---|
| 桌面：清单 → 托管 → 真插件 check/download 真验签 | `pwsh tools/updater-local-e2e.ps1` | **exit 0**，S1–S7 全 PASS（含「篡改 1 字节被拒」`InvalidSignature`、「错公钥被拒」`UnexpectedKeyId`） |
| 桌面：新私钥对新产物重新签名 | `tauri signer sign -k <key> -p <pw> AudioLink_0.1.0_x64-setup.exe` | 新 `.sig` 产出；旧密钥那份留档为 `.sig.oldkey` |
| Android：双 ABI release 用新 keystore 签出 | `pwsh tools/gradlew.ps1 -JavaHome <JDK 17+> :app:assembleRelease` | **BUILD SUCCESSFUL**；`apksigner verify` 对两个包都报 `V2 Signer` + `CN=AudioLink Release` + 指纹一致 |
| 许可判定含 Android 一栏 | `pwsh tools/license-audit.ps1 -Report <临时路径>` | exit 0：Rust 651 / 前端 154 / Android 114 包 · denied 0 · notice 18 |

### 14.6 仍然必须人工（边界没变）

| 项 | 为什么 |
|---|---|
| **Windows 代码签名证书** | 外部资产（购买 + 实名）。它与更新签名是**两回事**：它消除 SmartScreen 警告，不参与 updater 验签 |
| **GitHub Release 正式发布**（草稿 → Publish） | 对外不可逆；`releases/latest` 只认正式 Release |
| **干净机器装 0.1.0 → 升到 0.1.1** | 会改系统状态（Program Files / 注册表）；「干净」本身是环境属性 |

⇒ 这三件做完，M5 的发布链路才算真正收口；**代码侧与 CI 侧本轮已无已知缺口**。

### 14.7 顺带更正 §13.1（避免下一轮扫描再当缺口捞出来）

| §13.1 原表述 | 判定 | 实况 |
|---|---|---|
| #1「CI 生成的声明可能不含 Android 一栏」 | **已处置** | 走的是「明确失败」路线（`0bf3f56`），本轮再把采集接进两处 workflow（§14.3）—— 两条路都收口了 |
| #2「`ci.yml` 那一步名字仍叫 License audit (Rust + frontend)」 | **过时** | 现名已含「Android 一列见 tools/android-licenses.ps1」 |
| #3「报告模板里『未覆盖 Android 投放位置』是自动生成的」 | **仍成立** | 属工具输出口径，未动；改它要动 `audiolink-tools::license` 的渲染函数 |

### 14.8 CI 端到端验证（同日晚些时候，run `35483230945`）

`gh workflow run release.yml`（`publish=false`，只构建不建 Release）：

| job | 结果 | 关键步骤 |
|---|---|---|
| `android apk` | ✅ **3m56s** | 依赖树 + 许可采集（§14.3 新增）· `> Task :app:assembleRelease`（**真 release 路径**，不再是降级 debug）· 两个 artifact 上传 |
| `desktop installer` | ✅ **21m41s** | `Generate third-party notices`（**曾经的必红点**）· `Tauri build`（**带更新签名**）· `Generate update manifest` · 同源校验 · **`Updater end-to-end`** |
| `publish release` | skipped | 符合设计：`publish=false` 不建 Release |

产物：`audiolink-desktop` 11.2 MB（安装包 + `.sig` + `latest.json` + 绿色版）、`audiolink-android` 6.7 MB（双 ABI release APK）、`android-licenses` 1.4 KB。

同批 `ci.yml`（run `35483230336`）**恢复全绿** —— 顺带清掉了 core-light 自 `85c57f9` 起一直红的 rustfmt 问题
（`desktop/src-tauri/src/settings.rs` 两处，`0375581`）。**这两处无关本次改动，但 CI 红着谈「发布链路可用」没有意义。**

### 14.9 第一次跑就踩到第二个真缺陷：`Join-Path` 不会丢弃第一段

`release.yml` 的 android job **第一次运行就红**（run `35482996973`）：

    Gradle 缓存不存在：/home/runner/.gradle/caches/modules-2/files-2.1

实况在 `tools/android-licenses.ps1`：`Join-Path $root $Cache` —— PowerShell 的 `Join-Path` 在第二段是
**绝对路径**时**不会**丢弃第一段（与 .NET `Path.Combine` 的行为不同），于是把它拼成 `<repo>//home/runner/...`；
而报错又打印原始参数 `$Cache`，把「拼接出错」伪装成「缓存不存在」—— 排查时白烧一轮 CI。

- 修法：新增 `Resolve-RepoPath`（绝对路径原样使用，相对路径才拼仓库根，覆盖 report/cache/out 三处），报错改为打印**实际检查的路径**（`09e2b25`）；
- 为什么现在才暴露：这个缺陷**只在传绝对路径时出现**，而本地一直传相对路径（`.gradle-home/...`）；新加的 CI 步骤是它的第一个绝对路径调用方；
- 本地三例验证：相对路径 exit 0 / 绝对路径 exit 0 / 不存在的路径报出真实路径。

> **Android 侧的应用内更新**（独立于 Tauri updater 的机制、安全模型与实现）见 `docs/61-android-self-update.md`。

> **教训（写给下一轮）**：新接进 CI 的脚本，第一次跑要当作「它还没被验证过」——
> 两个缺陷（声明缺 Android、`Join-Path` 绝对路径）都是**第一次真实运行**才现形的。
