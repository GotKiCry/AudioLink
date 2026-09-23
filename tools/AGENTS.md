# AGENTS.md — tools(PowerShell 运维脚本)

以 pwsh 脚本为主(另有 1 个 Node 脚本 `check-update.mjs` 与 1 个 Rust 探针 crate `updater-plugin-probe/`)。`.ps1` 的 CRLF 行尾由 `.gitattributes` 强制,勿转 LF。证据产出落 `target/evidence/`(不入库),结论写进对应 `docs/NN-*.md` 与看板。

| 脚本 | 干什么 |
|---|---|
| `gradlew.ps1` | Gradle 包装:自动处理 JDK/ANDROID_HOME,Android 构建唯一入口 |
| `check-version.ps1` | 三端版本与根 `Cargo.toml` 一致性,发布前必跑 |
| `package-android.ps1` | Android 发布包一条命令(含四环同源验收),产物到 `out/` |
| `license-audit.ps1` | 三端依赖许可审计;`-Notices` 生成 `docs/compliance/THIRD-PARTY-NOTICES.md` |
| `android-licenses.ps1` | 采集 Android(Gradle/Maven)依赖许可,产出供审计判定 |
| `soak-report.ps1` | soak JSON → 人读报告;`depth_drops`(主动丢帧)与 `late_drops`(迟到)分列 |
| `sync-board.ps1` | GitHub Projects 看板同步,按标题幂等;先 `-DryRun` 再真跑(纪律见 CONTRIBUTING §7) |
| `acoustic-latency.ps1` | 声学出声延迟量具(量法见 docs/55 §2、docs/60) |
| `tauri-build.ps1` / `tauri-portable.ps1` / `tauri-latest-json.ps1` | 桌面发布:NSIS(有私钥时带更新签名)/ 便携 zip / 更新清单 latest.json |
| `check-update.mjs` | 本地校验 latest.json 签名与安装包是否配对 |
| `updater-local-e2e.ps1` / `updater-plugin-probe/` | 自动更新端到端脚手架;后者是驱动真 tauri-plugin-updater 的探针 crate |

新增脚本惯例:头部注释写「为什么存在」与历史教训(沿用现有脚本风格);有环境前置(JDK、设备、私钥)的在脚本内自检并给人话报错,失败要响亮。
