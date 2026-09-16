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
