# CI 关键路径实测与削减（core job）

> 日期 **2026-09-16**。一句话：先把 core job 的时间**量出来**，结果发现看板上两条 CI 待办的
> 前提数据都已过期（依赖被 cache 吃掉 / 安装早就只剩 1 s）；于是把刀口对准真正的关键路径 ——
> **workspace 本地 crate 的编译与测试二进制链接**，并诚实记录收益到底有多少。

---

## 1. 量：单次 run 的分步耗时

数据源：run `35079292910`（提交 `87cb059`），`gh run view --json jobs` 的步骤时间戳。

| job | 总时长 | 分步 |
|---|---|---|
| **core**（墙钟关键路径） | **236 s** | checkout 6 / rust-toolchain 10 / rust-cache 53 / fmt 3 / clippy 25 / **test 139** |
| android | 192 s | cache 17 / cargo-ndk 安装 **1** / ndk build 31 / **assembleDebug 130** |
| desktop | 102 s | cache 38 / pnpm 4 / 前端 build 4 / cargo check 25 |
| version-consistency | 5 s | 版本一致性脚本 |

CI 墙钟 = **max(各 job) = core 的 236 s**，所以只有 core 值得动。

## 2. 根因：依赖不在关键路径上

两个证据都来自同一次 run 的日志：

1. `Swatinem/rust-cache` 打印 `Cache hit ... full match: true`；随后 test 步骤里的
   **`Compiling` 行只有 9 个 workspace 本地 crate**（types / proto / identity / net / audio /
   engine / discovery / tools / ffi），quinn / rustls / tokio **一行都没有**
   → QUIC 依赖整棵树被 cache 完整复原，**每次 run 都不付这笔钱**。
2. test 步骤 139 s 中，把所有 `test result: ... finished in X` 的 X 加起来 = **39.2 s**（真正的测试运行），
   其余 ~100 s 是本地 crate 编译与**每个 target 链接一个测试二进制**。

**与看板旧数据的差异**：

| 看板待办 | 旧前提 | 实测 |
|---|---|---|
| [CI] QUIC 依赖 feature 门控（回收 core job 的 ~2.5 min） | tools 引入 quinn/rustls/tokio 后 core 由 1m36s 涨到 4m10s | 那是**引入当天的一次性冷编译**（cache miss）。此后 cache 命中，依赖零成本 → 门控在这里**没有可回收的时间**；而 core 的 clippy 必须保留 `--all-features` 的 lint 覆盖，把 QUIC 藏进 feature 只会让「没被 lint 的组合」变多 |
| [CI] Android job：预编译 cargo-ndk（省 ~2 min） | 每次 run 重编 cargo-ndk | `cargo install cargo-ndk --locked` 实测 **1 s**：rust-cache 默认缓存 `~/.cargo/bin`，二进制本来就在 cache 里 → 前提同样失效。android 的真关键路径是 Gradle（130 s） |

## 3. 改：刀口对准编译

| 改动 | 位置 | 为什么 |
|---|---|---|
| 5 个无 `#[test]` 的 bin 标 `test = false` | `core/crates/audiolink-tools/Cargo.toml` | alp2-dump / latency-probe / link-loop / soak-runner / netem-sim 里没有测试，却各自链接一个测试二进制。**device-link 与 self-loop 里有测试，原样保留** |
| core 的 test 步骤加 `--lib --tests` | `.github/workflows/ci.yml` | 内核各 crate 的 doctest 计数**全是 0**（9 行 `Doc-tests` 全 0 项），为它们链 rustdoc 是纯开销 |
| android job 缓存 `~/.gradle/{caches,wrapper}` | `.github/workflows/ci.yml` | assembleDebug 130 s 是该 job 的关键路径 |

**测试零损失（本地实测）**：改动前 `cargo test --workspace --exclude audiolink-desktop` = 41 个 suite / **355 passed**；
改动后 `--lib --tests` = 27 个 suite / **355 passed**。差的 14 个 suite 恰好 = 9 个 0 项 doctest + 5 个 0 项 bin harness。

## 4. 验证

- 本地：`cargo fmt --all --check`、`cargo clippy --workspace --exclude audiolink-desktop --all-targets --all-features -- -D warnings`、
  `cargo test --workspace --exclude audiolink-desktop` 全绿（355 项一项不少）。
- CI：提交 `6ae748b` → run `35080728329` 四个 job 全绿。

## 5. 改后的数字（诚实版）

| 步骤 | 改前 `35079292910` | 第 1 次 `35080728329` | 第 2 次 `35081356931`（稳态） | 说明 |
|---|---|---|---|---|
| **core job 总计** | 236 s | 368 s | **256 s** | 墙钟关键路径；第 1 次的 368 s 是一次性代价（见下两行） |
| core / clippy | 25 s | 92 s | **22 s** | 第 1 次改了 `Cargo.toml`，target 里的 clippy check 产物失效 → 重编一次；第 2 次回到稳态 |
| core / test | 139 s | 138 s | 167 s | **削减 bin harness 与 doctests 的收益小于 run 间波动**（139 → 138 → 167 s），真正的测试运行只有 39 s |
| **android job 总计** | 192 s | 243 s | **85 s** | **↓107 s** |
| android / assembleDebug | 130 s | 150 s | **16 s** | Gradle 缓存生效：`actions/cache@v4` 从首轮 miss（0.0 s）变成命中（6 s） |
| android / cargo-ndk 安装 | 1 s | 1 s | 0 s | 本来就被 rust-cache 覆盖 |

**结论（不美化）**：

- **有实测收益的只有 Gradle 缓存**：android job 192 → 85 s（assembleDebug 130 → 16 s）。
  这正是「预编译 cargo-ndk 省 ~2 min」想要的 2 分钟，只是**该省的地方在 Gradle，不在 cargo-ndk**。
- **core 没有可测收益**：139 → 138 → 167 s，波动大于改动本身。本地确实少了 14 个测试二进制
  （41 suite → 27 suite / 仍是 355 项），但这层开销在 CI 上被噪声淹掉。
- 两条旧待办的**前提都被证伪** —— 这是本轮最值钱的部分：依赖不是每次 run 都付费（cache 全命中），
  cargo-ndk 也不是每次重编（1 s）。

## 6. 后续（还有哪里能压）

- core 的 139–167 s 是「9 个 crate 编译 + 27 个测试二进制链接」的地板价，Windows 链接尤其贵。
  想真正缩短反馈，只剩**并行拆分**：把 types/proto/identity 这类轻内核拆成独立的快反馈 job，
  重内核（net/engine/ffi/tools）另一个 job —— 墙钟仍由重 job 决定（≈3.5 min），收益在「协议改动的反馈时间」。
- android 若还要压，继续往 Gradle 上做（`--configuration-cache`、只跑必要 task）。
- rust-cache 的恢复固定要 53 s（core）/ 45 s（desktop），属缓存粒度问题，改动收益与风险都不小，先记账。

---

## 7. 再拆一次：core → core-light / core-heavy（2026-09-16 第二轮）

§6 说的「只剩并行拆分」这一轮做了。理由是依赖深度**实测差异很大**（`cargo metadata`）：
- `audiolink-types` 无任何内部依赖；`audiolink-proto` / `audiolink-identity` 只依赖 `types`；
- `audiolink-engine` 依赖 `audio` + `identity` + `net` + `proto` + `types`；`ffi` / `tools` 拉全栈。

所以协议层（golden vectors 就在 proto）完全不必等 QUIC 与音频栈编完。

| job | 命令要点 |
|---|---|
| `core-light` | `fmt` + clippy/test 只针对 `audiolink-types` / `-proto` / `-identity` |
| `core-heavy` | `--workspace --exclude audiolink-desktop --exclude audiolink-types --exclude audiolink-proto --exclude audiolink-identity` |

**重 job 用 `--exclude` 而不是逐个 `-p`**：将来新增 crate 会**自动**落进重 job。漏测一个 crate 比 CI 慢贵得多。
覆盖正确性本地校验（`cargo metadata` 计算集合，非目测）：非桌面成员 9 个 = 轻 3 + 重 6，**交集为空**。

本地热缓存真跑一遍两组命令（不是估算）：

| 组 | fmt | clippy | test |
|---|---|---|---|
| light（3 crate） | 0.8 s | 2.7 s | 6.3 s |
| heavy（6 crate） | — | 1.7 s | **73.3 s**（27 个测试二进制，其中 engine 占 13 个） |

### 7.1 第一次 CI 运行（run `35097376811`）：冷缓存的代价，如实记录

| job | 总时长 | 分步 |
|---|---|---|
| `core-light` | **198 s** | cache 16 / clippy 75 / test 37 / post-cache 50 |
| `core-heavy` | **744 s** | cache 16 / **clippy 295** / **test 354** / post-cache 59 |
| android | 102 s | （既有） |
| desktop | 111 s | （既有） |
| version-consistency | 8 s | |

墙钟 **744 s**，比拆分前的 236 s **慢了 508 s**。原因不是拆分本身，而是 **`Swatinem/rust-cache` 的 key 含 job 名**：
两个新 job 各自冷启动，整棵依赖树（quinn / rustls / tokio / cpal 一行不少）要重编一次。这是「新增 job」的
**一次性成本**，稳态要看第二次运行。

但收益在第一次运行里已经能直接读出来：**`core-light` 198 s 全绿收工的那一刻，`core-heavy` 才刚走完 clippy（295 s）**，
协议层结论比另一个 job 早 **546 s** 到手。

## 8. 稳态（第二次运行，缓存按 job 各自命中后）

第二次运行 `35098695833`（缓存按 job 各自命中后，五个 job 全绿）：

| job | 总时长 | 分步 |
|---|---|---|
| `core-light` | **80 s** | cache 34 / fmt 2 / clippy 7 / test 13 |
| `core-heavy` | **297 s** | cache 32 / clippy 19 / **test 229** |
| android | 88 s | ndk build 36 / assembleDebug 17 |
| desktop | 149 s | cache 66 / cargo check 42 |
| version-consistency | 18 s | |

两次运行放在一起才回答得了「值不值」：

| 指标 | 拆分前 | 拆分后（稳态） | 变化 |
|---|---|---|---|
| 协议层（golden vectors）拿到结论 | 236–256 s | **80 s** | **−69%** |
| CI 墙钟 | 236–256 s | 297 s | +41~61 s（+20%） |
| job 数 | 4 | 5 | |

拆分的本质是**用墙钟换反馈**：重 job 要多付一份 rust-cache 恢复（32 s）与 checkout，所以墙钟反而长了。
立这笔交易的根据是「协议是跨端契约」—— 协议改动要三端齐改，错一次等 4 分钟是最贵的等待。

第一次（冷缓存）744 s 与本节的 297 s 相差 447 s，全部是依赖树重建：**拆分 CI job 的真实成本不是脚本行数，
而是缓存被切碎后的第一次运行**。

### 8.1 还剩什么

`core-heavy` 的 297 s 里 test 占 **229 s**，而真正的测试运行只有约 40 s（各 suite `finished in` 之和），
其余是 **27 个测试二进制各自的编译与链接** —— `audiolink-engine` 一个 crate 就贡献 13 个。
下一步的杠杆在这里（合并集成测试二进制，或按依赖再拆 job），已记入看板。

---

## 9. 修正上一节：链接不是主体，串行才是（2026-09-16 第三轮）

§8.1 写了一句「27 个测试二进制各自的编译与链接」是 229 s 的主体。**这句是错的**，本轮读 CI 日志把它纠正过来。

### 9.1 重新归因：把 229 s 摊开

从 run `35098695833` 的 core-heavy 日志里逐 suite 取 `finished in`：

| 成分 | 数值 | 依据 |
|---|---|---|
| 26 个 suite 运行时间之和 | **75.8 s** | 日志里 26 行 `finished in` 求和 |
| 最长单个 suite | **16.25 s** | `clock_sync` |
| 其余 | **≈153 s** | 编译 9 个本地 crate + 27 个测试 target |

另在本地单独量一个**空**的 engine 测试二进制（编译 + 链接）：**0.75 s**；把 13 个引擎测试二进制全重链一遍约 10 s。
→ **链接从来不是杠杆**。

### 9.2 真正的杠杆：`cargo test` 逐个二进制串行

`cargo test` 一次只跑一个测试二进制；26 个二进制互不依赖却排队。套件运行时间之和 75.8 s、最长一个 16.25 s，
差值就是排队浪费。`cargo nextest run` 跨二进制并行调度。

本地热缓存实测（heavy 集：workspace 去掉桌面外壳与三个轻内核）：

| 运行器 | 耗时 |
|---|---|
| `cargo test` | 78.2 s |
| `cargo nextest run` | **21.3 s** |

连跑三次：**18.8 / 18.8 / 18.8 s**，344 项全过、零 flaky。

**并且逐项核对过没有漏测**：同一条命令 `cargo test --lib --tests` 报 26 suite / **344 passed**，nextest 报 **344 passed**
—— 数量完全一致（跳过 doctests 是两边一致的既有约定）。

### 9.3 接进 CI

两个内核 job 的 test 步骤改用 `cargo nextest run`，并加 `.config/nextest.toml`：单个测试 60 s 告警、
连续 3 个周期不收敛即判失败，`fail-fast = false`（失败时看到全部面，而不是第一个就收摊）。

| job | 步骤 | 改前（`cargo test`） | 改后（`cargo nextest run`） |
|---|---|---|---|
| `core-heavy` | Test | 229 s | **199 s** |
| `core-light` | Test | 13 s | 26 s |

`core-heavy` 的 test 步骤 −30 s（−13%）。**本地热缓存是 78.2 → 21.3 s（−73%），CI 为什么只有 13%**：
CI 的 199 s 里约 **153 s 是编译**（受 4 核与 Windows 链接限制，本身不可并行），能并行的只有运行那部分
（76 → 约 46 s）。本地测的几乎全是运行时间，所以并行收益看起来大得多。**两个数字都对，说的是不同的事**。

`core-light` 反而从 13 s 涨到 26 s：它只有 5 个测试二进制、运行时间本就极短，nextest 的启动与元数据开销摊不平。

**首次运行的安装成本（如实记录）**：`cargo install cargo-nextest --locked` 在两个 job 里各花了 **404 s / 391 s**
（CI 上从源码编译）。于是首次运行 core-light 495 s、core-heavy 666 s，墙钟 **666 s** —— 与上一轮「拆分 job 切碎缓存」
是同一个教训：**新工具 / 新 job 的第一笔账，是它的安装与缓存重建**。稳态靠 rust-cache 的 `cache-bin`（缓存
`~/.cargo/bin`）把二进制留住 —— 同一机制已让 `cargo-ndk` 的安装实测为 0~1 s。

### 9.4 稳态：`cargo install` 的产物没被 bin 缓存留住，改用官方预编译包

第三次运行（`8ec344e`）实测：

| 方案 | 安装步骤耗时 |
|---|---|
| `cargo install cargo-nextest --locked` | 404 / 391 / **375 / 393 s**（四次运行一次都没降） |
| 官方预编译包 `get.nexte.st/<pin>/windows`（7.5 MB） | **2.0 s** |

**上一节我写的「稳态靠 rust-cache 的 `cache-bin` 把二进制留住」是错的**。同一机制让 `cargo-ndk` 的安装只要 1 s，
但对 `cargo-nextest` 四次运行都是 375~404 s —— 它没有命中。**按实测写，不按机制推测写**：与其去猜 key 为什么不同，
不如换成不需要这条缓存的安装方式。

预编译包由 nextest 官方分发，版本显式 pin，装完立刻 `cargo nextest --version` 校验（装坏了在这一步红，
不留给测试步骤去猜）。本地实测下载 1.9 s + 解压 0.4 s；CI 上该步骤 **2.0 s**。

**最终稳态（run `35103776301`，五个 job 全绿）**：

| job | 总时长 | 关键步骤 |
|---|---|---|
| `core-light` | **83 s** | cache 34 / fmt 2 / clippy 12 / install 2 / test 28 |
| `core-heavy` | **267 s** | cache 32 / clippy 21 / install 2 / **test 187** |
| android / desktop / version-consistency | 102 / 116 / 9 s | （既有） |

三项串起来看才诚实：

| 指标 | 最初（单个 core job） | 现在 | 变化 |
|---|---|---|---|
| 协议层拿到结论 | 236–256 s | **83 s** | **−66%** |
| CI 墙钟 | 236–256 s | 267 s | +11~31 s |
| heavy 的 test 步骤 | 139–167 s | **187 s**（拆分后 cargo test 时是 229 s） | nextest −42 s |

协议层快了一倍多，墙钟只多一点点（多付一份 rust-cache 恢复与 checkout）—— 这笔交易成立。

**本轮最值钱的不是这几个数字，是两次被实测否掉的前提**：一次是「链接是主体」（否），一次是
「bin 缓存能留住 cargo install 的产物」（否，而且是我自己刚写下的推测）。

---

## 10. 再压一次：引擎的集成测试合并成一个 harness（2026-09-16 第五轮）

§8.1 里我曾写「27 个测试二进制链接是主体」，§9 把它证伪了（单个链接只占约 0.75 s）。
那条待办因此带着一句「**动手前必须先量**」—— 这轮量完也做完了。

### 10.1 先量

清掉引擎产物重编：**15 个可执行文件（1 个 lib + 14 个集成测试）共 7.4 s**，平均每个约 0.5 s（本地）。
合并 14 → 1 理论上省 13 次「编译文件 + 链接」。

### 10.2 两个踩坑（都值得记住）

① **crate root 的 `mod` 解析基准是它自己所在的目录**：`tests/engine.rs` 里的 `mod x;` 找的是 `tests/x.rs`，
不是 `tests/engine/x.rs`。文件还留在 `tests/` 时，这条回退路径**悄悄生效** —— 结果是独立二进制与 harness
**同时**构建，同一批测试跑两遍（nextest 报 **182** 而不是 163）。

② 正确布局是 **`tests/engine/main.rs` 作 root、14 个文件放在它旁边** —— 目录里的 `main.rs` 也是合法的测试目标，
且它的模块基准就是那个目录。

### 10.3 结果

| 指标 | 合并前 | 合并后 |
|---|---|---|
| 测试二进制 | 15 | **2**（lib + engine harness） |
| 本地重编 | 7.4 s | **6.3 s** |
| **CI 的 core-heavy test 步骤** | **187–200 s** | **166 s** |
| 测试总数 | 437 | **437**（一个没少） |

单跑一个文件仍然可以：`cargo nextest run -p audiolink-engine --test engine -E "test(group_epoch)"`。

### 10.4 还剩什么

其他 crate 是同样的结构：`audiolink-proto` 4 个集成测试、`audiolink-tools` 4 个（后者依赖 engine + QUIC，
每个链接更贵）。同样的手法还能再压一点 —— 但收益随数量下降（4 → 1 只省 3 次链接），值不值要再量。
---

## 11. 长跑门禁（soak-gate）的账（2026-09-17 第 92 轮）

第 92 轮给 CI 加了「弱网档 + 120 s」的长跑门禁（**为什么不是严格档**：`docs/22` §14.2 有负载敏感证据）。
它按**独立 job** 加，与 core-light / core-heavy / android / desktop **并行**，
所以进的是「墙钟 = max(各 job)」这本账，而不是各 job 之和：

| 项 | 估算 | 依据 |
|---|---|---|
| checkout / toolchain / rust-cache | 6 / 10 / 53 s | §1 的分步实测 |
| build `soak-runner`（cache 命中）| 30–90 s | 依赖已在 cache；本地热缓存实测 4–8 s |
| soak 120 s + 启停 | ~125 s | 计划 120 s |
| **`soak-gate` 合计** | **≈ 225–285 s** | 与 `core-heavy`（≈267 s，§8）**同量级** ⇒ 稳态下不抬墙钟 |

**一次性代价**：`Swatinem/rust-cache` 的 key 含 job 名，新 job 首轮 cache 是冷的 ——
按 §2 的同类比（引入 QUIC 依赖当天 core 236 → 368 s），首轮预计多 2–4 分钟编译，次轮起消失。

**没做的事**：
- 没接 nightly 的 8 h 长跑（8 h 本来就放 CI 之外：第 88 轮结论，长时退化靠人工/nightly）；
- 没上 300 s 档位：那会把墙钟关键路径抬到 ~320 s（约 +1 分钟/PR）。真需要更长时长的档位，
  应作为**独立 nightly workflow** 加，而不是往 PR 的关键路径上加。

**未验**：真实 runner 上的读数 —— 本轮不 push、不触发 CI（也避免烧 CI 时间），CI 侧只做了静态检查
（YAML 用解析器验过 + 步骤脚本逐字复核）。**首轮跑完请把 `soak-gate` 的实际分步时长补进本表**，
并按需修正上面的估算。




