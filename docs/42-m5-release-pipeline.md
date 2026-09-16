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
| Android debug 包 | ✅ | CI `android` job：`cargo ndk`（arm64-v8a + armeabi-v7a）+ `assembleDebug` + artifact 上传。2026-09-17 起分 ABI 产出两个：`app-arm64-v8a-debug.apk` / `app-armeabi-v7a-debug.apk`（见 §9） |
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

绿色版没有「配置随身携带」这个语义：`settings.json` / `trust.json` / `identity/*.pem` 都在
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
  配置路径来自 `app_config_dir()`，与安装形态无关（现有的 `trust.json` / `identity/` 就在那里）；
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






