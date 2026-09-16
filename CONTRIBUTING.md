# 贡献指南

> 这份文档只写**这个仓库特有的约定** —— 通用做法（分支、PR 礼节）不重复。
> 第一次进来请按 §1 的顺序读，能省掉大量来回。

---

## 1. 先读什么

| 顺序 | 文档 | 为什么 |
|---|---|---|
| 1 | [`docs/00-overview.md`](docs/00-overview.md) → [`docs/01-requirements.md`](docs/01-requirements.md) | 需求是契约：**不要凭常识改动契约**（例子见 §4 末尾） |
| 2 | [`docs/02-architecture.md`](docs/02-architecture.md) → [`docs/03-protocol.md`](docs/03-protocol.md) | 协议是跨端契约，改一处要三端齐动 |
| 3 | [`docs/10-handoff.md`](docs/10-handoff.md) 的 §4.0 | 最新进展与「下一步」，比任何摘要都新 |
| 4 | [`docs/06-dev-environment.md`](docs/06-dev-environment.md) | 环境与构建（含一份「血泪清单」） |

---

## 2. 环境与构建

见 [`docs/06-dev-environment.md`](docs/06-dev-environment.md)。最短路径：

```powershell
cargo build --workspace                              # 内核
cd desktop; pnpm install; pnpm tauri dev             # 桌面端
pwsh android/scripts/build-rust.ps1                  # Android：先交叉编译 .so（双 ABI）
pwsh tools/gradlew.ps1 -JavaHome <JDK17> assembleDebug  # 再打 APK（本机 JAVA_HOME 可能是 11）
```

---

## 3. 改动前先想三件事

1. **这是不是跨端契约？** 协议帧、状态机语义、编解码参数都属于契约 —— 要同时改 Rust 内核、Kotlin 绑定/Android 侧与
   桌面端，并更新 [`docs/03-protocol.md`](docs/03-protocol.md) 与它的 golden vectors。
2. **有没有用户可见的行为变化？** 有的话，界面文案要走 `desktop/src/i18n.ts`（中英都要给），用户手册里对应的
   那一节也要跟着改（[`docs/manual/`](docs/manual/)）。
3. **这件事需要什么证据？** 交付物的验收方式写在它的 `docs/NN-*.md` 里；**拿不到的证据要写成「未验」清单**（§6）。

---

## 4. 门禁：提交前必须全绿

```powershell
cargo fmt --all --check        # ← 不是可选项。本地漏跑它，CI 一定会在 core-light 上红
cargo clippy --workspace --exclude audiolink-desktop --all-targets --all-features -- -D warnings
cargo nextest run --workspace --exclude audiolink-desktop --lib --tests
cargo clippy -p audiolink-desktop --all-targets --all-features -- -D warnings
cargo test -p audiolink-desktop
cd desktop; pnpm build          # tsc 严格 + i18n 一致性检查 + vite
```

Android 改动额外跑：`pwsh android/scripts/build-rust.ps1`（双 ABI）与 `pwsh tools/gradlew.ps1 -JavaHome <JDK17 路径> assembleDebug`；
发布相关改动跑一次 `pwsh tools/tauri-portable.ps1 -Verify`。

> **一个真实的教训**：曾经有一次提交本地只跑了 clippy 与 test、没跑 `cargo fmt`，CI 的 core-light 直接红。
> 门禁清单是按「漏一个就会在 CI 上花一轮」写的，别挑着跑。

**关于需求契约**：`docs/01-requirements.md` 里的 FR 编号就是契约。曾经把「双 ABI」按常识理解成
arm64 + x86_64，读到 FR-40 才发现写的是 arm64 + armeabi-v7a —— **先读契约，再动代码**；要改契约就单独提出来并写清理由。

---

## 5. 提交信息

Conventional Commits：`feat(scope): ...` / `fix(scope): ...` / `docs(scope): ...` / `chore(scope): ...`。
正文写**为什么**，不写「改了哪几行」（diff 已经写过了）。

```text
fix(tools): correct three soak judging defects found by the 8h weak-network run

--tolerant left the bitrate criterion on: transient bitrate swings 205-427 kbps against a
320 kbps target, so a weak-network run could never pass. ...
```

---

## 6. 代码与文档约定

| 约定 | 说明 |
|---|---|
| 注释写「为什么」 | 「这里为什么这么写」「不这么写会怎样」比「这行做了什么」值钱得多；暂时没想清的写成 `TODO(范围)` |
| 不用 `unwrap` / `expect` / `panic` | clippy 已设为 `-D warnings` 拒绝；测试模块里 `#![allow(...)]` 即可 |
| 界面文案 | 一律走 `desktop/src/i18n.ts`；`pnpm build` 会检查键集合与占位符一致 |
| 错误信息 | 人话 + 可操作（「打开设置失败：…」而不是 `Err(Store)`）；面向用户的串要双语 |
| 纯逻辑优先 | 能做成纯函数/纯状态机的（判定、协议编解码、混音）就别放 I/O；编排放 `bin/` 或集成测试 |
| 一个交付物一份文档 | `docs/NN-<主题>.md`：背景 → 做法 → 取舍 → 实测数字 → **未验清单** |
| 诚实记账 | 「未验」「已知取舍」「这次没给出通过」都要写进文档与看板，不要只写好消息 |
| 护栏要自证 | 新增校验/门禁时，给一次「故意改坏 → 变红」的证据（例：`desktop/scripts/check-i18n.mjs`） |

---

## 7. 看板

任务表在 [`tools/sync-board.ps1`](tools/sync-board.ps1)（PowerShell 数组，按**标题**幂等匹配）：

```powershell
pwsh tools/sync-board.ps1 -DryRun     # 先看会改什么
pwsh tools/sync-board.ps1             # 再同步
```

改完**必须远端核对**（脚本会报「新建/更新/说明更新」几条，但要自己确认）：

```powershell
gh project item-list 2 --owner GotKiCry --limit 300 --format json   # 数一遍状态分布
```

条目正文写**证据**：命令、数字、提交号、以及仍未验的部分。

---

## 8. 许可

本项目以 Apache-2.0 发布（[`LICENSE`](LICENSE) / [`NOTICE`](NOTICE)）。
新增依赖前先看许可；发布前跑一次：

```powershell
pwsh tools/license-audit.ps1 -Notices     # 生成 docs/compliance/THIRD-PARTY-NOTICES.md
```

被拒的许可（GPL 等）会让审计失败；不确定的判成 `notice` 并人工确认。详细规则见
[`docs/41-compliance-license-audit.md`](docs/41-compliance-license-audit.md)。
