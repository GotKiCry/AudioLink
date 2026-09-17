# M2 · soak-runner（回环长跑 + 指标采集 + 异常快照）

> 看板：`[工具] soak-runner（8 h 回环 + 指标采集 + 异常快照）`。日期 **2026-09-16**。
> 一句话：把「跑 8 h 不出故障」从「人盯着看日志」变成**可判定的产物** ——
> 逐秒采样 → 越界留快照 → JSON 报告 → **退出码即判定**。

---

## 1. 用法

```text
soak-runner run [--seconds 28800] [--frame-ms 20] [--report PATH] [--expected-bps auto] [--warmup-seconds 3] [--quiet]
                [--tolerant] [--max-underrun-pct N]
                [--netem-loss-pct N] [--netem-delay-ms N] [--netem-jitter-ms N] [--netem-bandwidth-kbps N] [--netem-seed N]
```

| 参数 | 含义 |
|---|---|
| `--seconds` | 观测时长，默认 28800（8 h）；CI 冒烟用 60 |
| `--frame-ms` | 帧长（10 / 20 / 40 / 60），默认 20 |
| `--report` | 报告路径，默认 `target/evidence/soak/soak-<unix>.json` |
| `--expected-bps` | 期望码率：`auto`（默认）= 由**本次实际** codec 配置推导（目标 × 冗余双发 2 份），并跟随自适应升降；数字 = 钉死（判定链路自检）；`0` = 不判码率 |
| `--warmup-seconds` | 预热秒数，默认 3（预热期的越界不判） |
| `--quiet` | 不打印每秒进度 |
| `--tolerant` | 弱网档判据：会话不断 + 掩盖帧 ≤ 总帧数 1% + **静音（欠载）≤ 计划总拍数 N%**（默认 1.00，**待产品确认**，见 §11）；不判迟到/NACK/瞬时丢包/码率 |
| `--max-underrun-pct` | 弱网档的静音（欠载）上限，百分比可小数（默认 1.00）；`0` = 不判。**仅弱网档有效** —— 严格档按绝对值 0 判，给它预算会直接报错 |
| `--netem-*` | 在两个 Engine 之间插入弱网中继（丢包 / 延迟 / 抖动 / 限速），M2 弱网验收见 `docs/24-m2-netem-sim.md` |

退出码：**0 = 无异常；1 = 有异常；2 = 用法 / 初始化失败**。

链路与 `link-loop` 同源：同一进程内起两个**真实 Engine**（node-a 发送 / node-b 接收），
走 127.0.0.1 的真实 QUIC，实跑一遍 §5 的 PIN 配对；合成源 + `NullPlayout`，**不出声也不采集声卡**，
因此可以无人值守长跑。

---

## 2. 判据：什么是「异常」

判据的口径是**回环稳态不该出现的东西** —— 在一条 127.0.0.1 的链路上它们都应当是 0，
任何非零值都是缺陷信号，不是噪声：

| 种类 | 触发 |
|---|---|
| `not_streaming` | 会话离开 streaming（状态变化时只记一条，避免每采样刷屏） |
| `underrun` | 播放欠载计数增加（引擎里这一拍**没帧就补静音**，所以它就是「静音拍数」）。严格档按**绝对值 0** 判；弱网档按**比例预算**判（见 §11） |
| `playout_concealment` | PCM 掩盖帧增加（真丢了包） |
| `packet_loss` | 丢包率超过阈值 |
| `late_drop` | 迟到丢弃增加 |
| `nack_retransmit` | NACK 重传请求增加（回环不该丢包，也就该一次都没有） |
| `bitrate_out_of_range` | 码率偏离目标超过 `--expected-bps` 的 ±30% |
| `stalled` | 连续 5 次采样码率为 0（链路挂住） |

每条异常都留一份**现场快照**：时刻、种类、人类可读细节、当时的完整遥测 ——
这就是「8 h 之后回来看，能一眼定位是哪一秒、哪一项开始坏」的东西。

---

## 3. 报告结构（JSON）

```json
{
  "tool": "soak-runner",
  "started_at_unix": 1789546392,
  "planned_seconds": 20,
  "frame_ms": 20,
  "expected_bitrate_bps": 320000,
  "summary": {
    "samples": 18, "violations": 0, "dropped_violations": 0,
    "verdict": "ok",
    "underrun_pct_x100": 6, "underrun_budget": 30, "max_underrun_pct_x100": 100,
    "max_underruns_per_sample": 2, "longest_silence_lower_bound_beats": 1,
    "bitrate_judged": true, "expected_bitrate_bps_final": 640000,
    "depth_drops": 0, "depth_drops_judged": false,
    "final_stats": { "bitrate_bps": 320800, "loss_pct_x100": 0, "underruns": 0,
                      "plc_count": 0, "late_drops": 0, "nack_count": 0,
                      "buffer_level_us": 20000, "rtt_us": 1848, "...": 0 }
  },
  "violations": [],
  "coarse": [ { "bucket": 0, "at_secs": 3, "stats": { "...": 0 } } ]
}
```

- 自适应目标的变更时间线：`codec_adaptations[]` 与 `summary.codec_adapted_count` / `codec_target_bps_final` —— 见 §13；
- 静音（欠载）判据的三个数：`underrun_pct_x100`（实际占比）/ `underrun_budget`（判据用的绝对预算）/
  `max_underrun_pct_x100`（预算的比例来源，0 = 这次没按比例给预算）—— 见 §11；
- `max_underruns_per_sample` 与 `longest_silence_lower_bound_beats`：最坏一秒补了多少拍静音、
  以及由它推出的**连续静音下界**（**不参与判定**，见 §11.5）；
- `expected_bitrate_bps` 是**起跑值**，判据实际用的是 `summary.expected_bitrate_bps_final`（跟随发送侧自适应），
  `expected_bitrate_source` 说明来路（`follow-encoder-target` / `fixed-cli` / `off`）；
- `depth_drops` 是「降档主动丢帧」的独立账（与 `late_drops` 不同因），当前**只可见、不判**（`depth_drops_judged: false`）；
- `violations`：留存上限 200 条（超出只计入 `dropped_violations`）—— 8 h 跑不该产出无限大的报告；
- `coarse`：每 60 s 一条粗采样（8 h 约 480 条），既能看趋势又不撑爆报告；
- 报告**手写字段**而不直接序列化 `StreamStats`，避免为一个报告把 `serde` feature 拉进内核依赖图。

---

## 4. 实测（20 s 冒烟 + 判定链路验证）

| 跑法 | 结果 |
|---|---|
| `run --seconds 20` | 逐秒 streaming；码率 312→321 kbps 收敛后稳定，丢包 / 欠载 / 掩盖 / 迟到 / NACK **全 0**；队列水位 20 ms、RTT 1.8 ms；`verdict = ok`、**exit 0**、报告 `target/evidence/soak/smoke.json`（750 B） |
| `run --seconds 8 --expected-bps 100000`（故意把目标写错） | 6 条 `bitrate_out_of_range` 快照（含「偏离 220.52%」细节）→ `verdict = failed`、**exit 1** |

第二条不是"制造故障"，而是验证**判定链路**本身：采样 → 判定 → 快照 → 报告 → 退出码，闭环可用。

---

## 5. 与真正 8 h 验收的关系

本工具**是那件事的载体**，不是那件事本身：

- 8 h（28800 s）长跑 = `soak-runner run`（默认参数即 8 h），建议放 CI nightly 或人工触发；
- 报告（JSON）直接作为验收产物：`summary.verdict == "ok"` 即「8 h 无崩溃 / 无静音 / 无漂移」的机器判据；
- 真机（PC→Android）长跑仍走 `target/evidence/acceptance/` 下的既有脚本（那一侧要连真机，不在本工具职责内）。

---

## 6. 验证清单

| 层 | 内容 |
|---|---|
| 单元（27 项） | 稳态零异常、计数器增量 → 快照（含 delta 文案）、丢包/码率/挂住判定、状态离开 streaming 只记一条、快照上限与 `dropped_violations`、每分钟粗采样、JSON 报告字段、摘要文本；**码率期望值**：推导含冗余份数、跟随自适应且仍有牙齿、跟随不打开判据、来路进报告；**静音（欠载）**：预算 = 比例 × 计划总拍数、比例四舍五入、8 h 基线在建议值下必红 / 在过渡闸门下放行、0 = 不判、严格档仍是绝对值 0、最坏一秒与连续静音下界、报告与摘要都带比例 |
| 端到端（人工，本条给出命令与数字） | 20 s 冒烟（exit 0）+ 目标码率写错（exit 1）；**弱网档静音预算对照**：同参数 60 s，默认 1.00% → 0.20% 判 `ok`，`--max-underrun-pct 0.1` → 越线判 `failed`（见 §11.6） |

```
cargo test -p audiolink-tools --lib soak
cargo run -q -p audiolink-tools --bin soak-runner -- run --seconds 60
```

---

## 7. 未做 / 后续

- **真正的 8 h 长跑**尚未执行（工具就绪；跑一次 8 h 是环境时间成本，不是代码问题）；
- **真实弱网**（5 Mbps / 2% 丢包 / 30 ms 抖动）不在本工具内 —— 它跑的是干净回环；弱网需要注入，
  可复用 `tests/nack_retransmit.rs` 里那个有损 UDP 中继的做法，属 M2 后续；
- 遥测面板（M2 的另一项交付物）仍待做，本工具只产出报告文件。

---

## 8. 判定复算：`tools/soak-report.ps1`

报告是**机器产物**，但长跑结束时人是隔着几小时回来看的 —— 只信 soak-runner 自己打印的那一行 summary 不够，
还需要一条**可复算、可入 CI、可回填看板**的通路：

```text
pwsh tools/soak-report.ps1 [-Path target/evidence/soak/soak-8h-netem.json] [-Json]
```

输出四块：判决与规模（计划时长 / 帧长 / 目标码率 / 粗采样桶数 / 违规数 / 丢弃溢出数）、
**违规按 kind 聚合**（次数 + 首次与末次出现秒数 + 一条样例）、终值遥测（码率 / 缓冲 / 时钟偏差 / 漂移 /
抖动 p95 / RTT / 丢包 / 迟到 / 欠载 / NACK / PLC）、以及一句人话判定。

退出码与 soak-runner 同口径：**0 = 通过；1 = 有违规或 verdict 为 failed；2 = 报告缺失 / 非 JSON / 缺 summary**。
`-Json` 输出同一份摘要的结构化版本，供 CI 或看板脚本回填。

复算口径与 §2 一致：**只有 verdict 为 ok 且 violations 为空才算通过**；
soak-runner 侧的对应条件是 violations 为空且 dropped_violations 为 0，本脚本把两者都打印出来，
避免「快照被丢满 200 条、看起来却只有 200 条违规」这类误读。

### 8.1 用已有报告自测（无需重跑 8 h）

| 输入 | 期望 | 实测 |
|---|---|---|
| `target/evidence/soak/smoke.json` | 通过 | ok / exit **0**（18 次采样、0 违规） |
| `target/evidence/soak/negative.json` | 不通过 | `bitrate_out_of_range x6  3s..8s` / exit **1** |
| 不存在的路径 | 报告缺失 | exit **2**，stderr 给出绝对路径 |
| 非 JSON 文件 | 不可解析 | exit **2**，stderr 给出解析错误 |

### 8.2 与 `target/evidence/verification/soak-verify.ps1` 的分工

- `soak-verify.ps1`：解析**真机 30 min 文本日志**（逐行正则），复算水位 / 欠载 / 迟到的 P50/P95/P99 分位数；
- `soak-report.ps1`：读 **soak-runner 的 JSON 报告**，做违规聚合与终值摘要，不假定日志格式。

两者都不重跑链路 —— 长跑产物是唯一输入，判定可离线复算。

---

## 9. 8 h 弱网长跑的收获：三个判定缺陷（2026-09-16/17 实测）

一次真实的 8 h 弱网长跑（28800 s，5 Mbps / 2% 丢包 / 15±15 ms 抖动，`--tolerant`）：
采样 28798 次、粗采样 481 桶、**7055 条异常，全部是同一类** `bitrate_out_of_range`（集中在 2692..8293 s）。
判定 `failed`，但根因不在链路，而在**判定器自己**：

1. **弱网档漏关码率判据**：`--tolerant` 放宽了欠载 / 迟到 / NACK / 瞬时丢包 / 掩盖，却把码率留在原样。
   弱网下瞬时码率本就随自适应与重传摆动（实测 205 kbps ~ 427 kbps，目标 320 kbps），
   于是弱网验收永远红。修法：档位集中到 `SoakThresholds::weak_network(planned_secs, frame_ms)`。
2. **`0 = 不判` 与实现相反**：字段文档写着「0 = 不判」，而判定式 `deviation > tolerance` 在 0 时
   退化成「任何偏离都算异常」= **全判**。已加显式 guard，并补回归测试 `bitrate_tolerance_zero_means_dont_judge`。
3. **快照「先到先得」吃掉了后半程证据**：总量配额被刷得最凶的那一类独占 ——
   留存的 200 条现场全落在 t=2692..8293 s，**后面 5.7 h 一条证据都不剩**，
   而长跑最该回答的问题恰好是「现在还在不在发生」。修法：**配额按类给**（默认每类 25 条），
   并把 `violations_by_kind` / `last_violation_at_secs` / `last_violation_kind` 写进报告 ——
   计数与「最后一次」是完整事实，不受配额截断影响。

### 9.1 同参数 90 s 对照（可复现）

```text
cargo run -q -p audiolink-tools --bin soak-runner -- run --seconds 90 \
  --netem-loss-pct 2 --netem-delay-ms 15 --netem-jitter-ms 15 --netem-bandwidth-kbps 5000 \
  --tolerant --quiet --report target/evidence/soak/tolerant-90s.json
```

| 档位 | 实测 |
|---|---|
| `--tolerant` | 掩盖 8 帧（上限 45）→ **0 异常 / `ok` / exit 0** |
| 同参数去掉 `--tolerant` | 18 条异常（late_drop 7 / underrun 5 / packet_loss 3 / playout_concealment 3）→ **`failed` / exit 1** |

对照的意义：放宽**只发生在宽容档**，零容忍档的判据一点没被削弱。

### 9.2 8 h 报告的关键数字

| 项 | 值 |
|---|---|
| 采样 | 28798 次（计划 28800 s） |
| 异常 | 7055 条，全部 `bitrate_out_of_range`；留存 200、未留存 6855 |
| 掩盖 | 950 帧 / 约 1.44 M 帧 = **0.066%**（弱网档上限 1%） |
| 欠载 / 迟到 / NACK | 4307 / 4414 / 77（弱网档不判，记录在案） |
| 末次遥测 | 码率 349056 bps、水位 120 ms、RTT 41.8 ms、抖动 p95 26.1 ms、漂移 −4 ppm |
| 注入自账 | 观察 2551069 / 转发 2499542 / 按丢包率丢 51527（**2.01%**），与 `--netem-loss-pct 2` 相符 |

两处必须说清楚的保留：① **这次 8 h 跑没有给出「通过」** —— 修完上面两个判据缺陷后需要重跑一次，
才谈得上「8 h 弱网通过」；② 长跑期间这台机器同时在跑编译与测试（CPU 争用），因此欠载 / 迟到的绝对值偏保守。

### 9.3 报告字段（新增）

```json
"summary": {
  "violations": 200, "violations_total": 7055, "dropped_violations": 6855,
  "violations_by_kind": { "underrun": 4307, "bitrate_out_of_range": 7055 },
  "last_violation_at_secs": 8293, "last_violation_kind": "bitrate_out_of_range",
  "violation_limit_per_kind": 25
}
```

旧报告（无这些字段）仍可被 `tools/soak-report.ps1` 解析 —— 脚本只在字段存在时多打印一段「按类 + 最后一次」。

---

## 10. 读报告时的坑：`soak-8h-netem.json` 是**修复前的证据**，不是当前判定（2026-09-17 第 69 轮补）

第 69 轮复查时踩了一次：看到 `target/evidence/soak/soak-8h-netem.json` 里 `verdict: "failed"`、
200 条违规（**全部** `bitrate_out_of_range`）、`dropped_violations: 6855`，差点当成「当前代码的长跑失败」。
它其实就是 §9 描述的那一次 —— 修复**之前**的 8 h 跑，留在这里是当缺陷证据用的。

怎么一眼分清：

| 判据 | 修复前的报告 | 当前代码 |
|---|---|---|
| 违规类型 | 清一色 `bitrate_out_of_range`，`dropped_violations` 是数千量级 | 宽容档下码率违规**应当为 0** |
| 摘要字段 | 没有 `violations_total` / `violations_by_kind` / `last_violation_kind` | 这些字段齐全（§9.3） |
| 档位自述 | 日志写「弱网（宽容）：只钉会话不断与掩盖比例 ≤ 1%」，违规里却有码率 | 同一句自述，违规里不会有码率 |

**同参数对照（可复现）**：用当前二进制、完全相同的参数再跑 90 s 宽容档：

```text
异常：共 0 条（留存 0 条，未留存 0 条）→ 判定 ok
verdict=ok violations=0 dropped=0 samples=88
```

0 违规 vs 修复前的 7055 —— 这就是那次修复最直接的对照。报告也留在
`target/evidence/soak/tolerant-90s-current.json`。若哪天 `soak-8h-netem.json` 被重新生成，
请连同 `started_at_unix` 一起看：它当前是历史证据，不是体检结果。
---

## 11. 欠载（静音）判据：口径、现状基线、建议值与代价（2026-09-17 第 90 轮补，**待产品确认**）

### 11.1 为什么补这条判据

验收写「8 h 无崩溃/**无静音**/无漂移」「2% 随机丢包下可懂、**无长断音**」，
而引擎里 `underrun` 的语义就是「这一拍没帧 → **补静音**并计数」（`runtime.rs` 播放环的 `Hold` / `missed` 分支）。
旧版弱网档把 `max_underruns` 设成 `u32::MAX`（完全不判）——「8 h 无静音」这条验收因此被**制度性放开**：
8 h 弱网实测 153762 次静音，判定照样是 `ok`。本轮把这条判据补上：**让静音程度可判、可见、有据**。

### 11.2 口径（只从现有遥测推，不碰协议）

- **静音总占比** = 累计 `underruns` ÷ **计划总拍数**；
- **计划总拍数** = 计划秒数 × 1000 ÷ 帧长（8 h / 20 ms → **1 440 000** 拍）；
- 阈值字段 `SoakThresholds::max_underrun_pct_x100`（百分比 ×100，**0 = 不判**）；
  构造档位时折算成绝对预算 `max_underruns`，判定仍走原有的「累计值增量」路径（`check_underruns`）——
  与「换一条判定路径」相比，这条更小的改动同时保证了严格档的语义不变（严格档 `max_underruns = 0`，按绝对值判）；
- 三个数都进报告：`summary.underrun_pct_x100` / `summary.underrun_budget` / `summary.max_underrun_pct_x100`；
- `StreamStats` 是 0x13 的**冻结**载荷，本判据**没有**为它加字段（旧节点会解码报错，第 86 轮踩过）。

### 11.3 实测现状（建议值的依据）

| 跑法 | 报告 | 累计欠载 | 计划总拍数 | 静音占比 | 每分钟增量峰值 |
|---|---|---|---|---|---|
| 8 h 弱网（2% 丢包 / 15+15 ms / 5 Mbps，宽容档）**首次** | `soak-8h-netem.json` | 4307 | 1 440 000 | **0.30%** | 497 拍 |
| 同上，**重跑**（同参数） | `soak-8h-netem-rerun.json` | 153762 | 1 440 000 | **10.68%** | **1836 拍** |
| 干净回环 900 s 严格档（机器安静） | `strict-clean-900s.json` | 0 | 45 000 | **0.00%** | 0 |
| 干净回环 900 s 严格档（与构建同时跑） | `strict-clean-900s-autobps.json` | 29 | 45 000 | **0.06%** | — |
| 弱网 60 s 宽容档（同命令，**机器安静**） | 控制台（§11.6 第 1 行） | 6 | 3 000 | **0.20%** | 1 拍/s |
| 弱网 60 s 宽容档（同命令，**机器有负载**） | `tolerant-60s-underrun-pct1.json` | 280 | 3 000 | **9.33%** | 57 拍/s |

两次 8 h 差 35 倍**不是链路退化**，而是分布不同：首次的增量摊在全场（max 497 拍/min），
重跑是**集中爆发**（max 1836 拍/min ≈ 那一分钟 61% 的拍在补静音），且两者的最后 10 分钟都归零。
这与 §9.2 留的那条「长跑期间机器上有其它负载」一致。
⇒ 静音占比与严格档的 `late_drop` 有**同一条前提**：**要拿它做验收，就得独占机器**。

### 11.4 建议值（**待产品确认**）

| 口径 | 值 | 含义 | 代价 |
|---|---|---|---|
| **建议值**（验收意图） | **1.00%** | 「静音总占比 ≤ 1%」，与弱网档已钉死的「掩盖帧 ≤ 总帧数 1%」同量级 —— 欠载（补静音）与 PLC（掩盖帧）是「这一拍没有真音频」的两半 | 安静机器上的两次 8 h 弱网实测：首次 0.30% 过、重跑 10.68% **不过**。即它会**红**，因为那份重跑本身被争用污染了 —— 这是真话，不是判据过严 |
| **过渡闸门**（不退化） | **12.00%** | 现状基线 10.68% 之上留余量，只承诺「不比现在更差」 | 今天就能过，但把「约 10% 的时间在补静音」制度化；**不**承诺可听 |

代码里对应 `WEAK_UNDERRUN_PCT_X100_SUGGESTED = 100`（弱网档默认）与
`WEAK_UNDERRUN_PCT_X100_TRANSITION = 1200`；CLI `--max-underrun-pct N` 可在**不改代码**的前提下试跑任何值
（`0` = 不判）。**这两个值都只是建议，产品定案前不写进验收表**；严格档**不接受**这条预算 ——
`--max-underrun-pct` 与严格档一起用会直接报错（避免悄悄放宽「干净回环零静音」）。

### 11.5 遗留：「长断音」（连续静音）推不出来

「静音总占比」与「长断音」是两件事。现有遥测只有**累计计数**，且接收侧 1 Hz 才发布一次快照 ——
**推不出**「最长连续静音」。能给的只有一条**下界**：
一秒内 k 拍静音、该秒共 B 拍，静音至多被非静音切成 `B − k + 1` 段，于是最长连续静音 ≥ ⌈k ÷ (B − k + 1)⌉ 拍。
报告因此只给**最坏一秒**的静音拍数（`summary.max_underruns_per_sample`）与由它算出的下界
（`summary.longest_silence_lower_bound_beats`），并明确标注**下界**（真实只会更长）、**不参与判定**。
真正的连续性指标需要在引擎播放线程里直接统计「最长连续静音拍数」，并经**事件**（不是 `StreamStats`）上报 —— 记为遗留。

### 11.6 短跑对照：新判据真的会参与判定（该红就红）

同一组参数（60 s / 2% 丢包 / 15+15 ms / 5 Mbps / `--tolerant`）跑了三次，只有**预算**与**被机器负载影响的实测值**不同：

| 跑法 | 报告 | 实测 | 预算 | 判定 |
|---|---|---|---|---|
| 默认 1.00% | 控制台（机器安静） | 6 拍 = 0.20% | 30 拍 | **ok**（判据生效、没越线） |
| 默认 1.00% | `tolerant-60s-underrun-pct1.json` | 280 拍 = **9.33%** | 30 拍 | **failed**（`underrun×9`，首次越线 t=4s） |
| `--max-underrun-pct 0.1` | `tolerant-60s-underrun-pct0p1.json` | 12 拍 = 0.40% | 3 拍 | **failed**（`underrun×4`） |

第 1/3 行是「判据真的参与判定」的直接证据：**同一组参数**，只把预算从 30 拍改成 3 拍，判定就从 ok 变 failed；
快照细节自带口径 —— `欠载 +1（累计 4；上限 3 拍 = 计划总拍数的 0.10%）`。
第 2 行说明另一件事：这条判据会**如实反映机器争用** —— 同一命令在机器有负载时静音占比从 0.20% 涨到 9.33%
（增量集中在 t=35..38 s，最坏一秒 57 拍）。它不是「过严」，而是「8 h 无静音」这条验收**必须独占机器**的又一份证据
（与严格档 `late_drop` 零容忍同一条前提，见 §11.3 末段）。

---

## 12. 抖动口径：验收表的「30 ms 抖动」= ±15 ms（2026-09-17 第 90 轮钉死）

三份 8 h / 90 s 弱网报告注入的参数都是 `--netem-delay-ms 15 --netem-jitter-ms 15`，
而 `docs/05-roadmap.md` 的 M2 验收写的是「5 Mbps / 2% 丢包 / **30 ms 抖动**」。二者关系一次写清：

- `audiolink-tools::netem` 的抖动语义是「实际单向延迟在 `delay_ms ± jitter_ms` 之间**均匀**取值」
  （`netem.rs` 的 `NetemConfig::jitter_ms` 文档与实现），所以 `--netem-jitter-ms 15` = 抖动幅度 **±15 ms**，
  **峰峰 30 ms**；单测里也按这个口径写（`±15 ms 覆盖 30 ms 抖动`）；
- 验收表里的「30 ms 抖动」指的正是这个**峰峰值**，**不是** ±30 ms；
- `--netem-delay-ms 15` 是**固定单向延迟**（均值），与抖动是两回事：报告口径 = 15 ms 延迟 + ±15 ms 抖动。

**后续统一写法**：报告与看板一律写
`--netem-loss-pct 2 --netem-delay-ms 15 --netem-jitter-ms 15 --netem-bandwidth-kbps 5000`，
需要引用验收口径时写「30 ms 抖动（峰峰）= ±15 ms」，不再单独出现裸的「30 ms 抖动」字样。
---

## 13. 自适应目标（`CodecAdapted`）：可见性与降级分支的实测（2026-09-17 第 91 轮补）

### 13.1 为什么记它

「自适应生效」以前只有 `EngineEvent::CodecAdapted` 一条时间线，而桌面端**只把它写进日志** ——
报告与界面上都看不出「目标码率到底动过没有」。soak-runner 现在把这条时间线**落进报告**。

### 13.2 口径：目标码率 ≠ 实测码率

- **目标码率**（自适应算出来的那个数）才是这个部件的**动作**；
- 接收侧**实测链路码率** ≈ 目标 × 冗余双发 2 份，还混着重传 / 突发的噪声 —— 不能当动作读。

| 报告字段 | 含义 |
|---|---|
| `codec_adaptations[]` | 时间线：`at_secs` / `from_bps` / `to_bps` / `reason`（引擎事件原文） |
| `summary.codec_adapted_count` | 变更次数（**0 = 全程没动**，这本身是个结论） |
| `summary.codec_target_bps_final` | 末次目标（0 = 未观察到变更） |
| `summary.expected_bitrate_bps_final` | 判据用的期望值（= 目标 × 2；`--expected-bps auto` 时跟着目标走） |

控制台摘要多一行「自适应目标：变更 N 次（降级 X / 恢复 Y），末次目标 Z bps」，并列出前 10 条时间线。

### 13.3 丢包档扫描：几档注入才够触发降级（60 s / 宽容档 / `--netem-*`）

| 注入 `--netem-loss-pct` | 接收侧 `loss_pct`（粗采样 t=3 / 末值 / 降级事件里的读数） | 自适应 | 目标时间线 |
|---|---|---|---|
| **2%** | 0 / 0 / —— | **0 次**（不降级） | —— |
| 10% | 0 / 1.92% / 2.04% | 2 次 | t=6s 160000→128000、t=21s 128000→102400 |
| 20% | 4.00% / 7.31% / 12.00% | 3 次 | t=4s、7s、10s → **96000（§8 下限）** |
| 30% | 7.40% / 25.86% / 34.21% | 6 次（两轮） | t=3/6/9s + t=23/26/29s |
| 50% | 33.00% / 0 / 37.50% | 3 次 | t=8s、11s、17s → 96000（会话还抖了一下：`not_streaming×1` / `stalled×1`） |

**结论**（复现了第 88 轮的判断）：**2% 注入下接收侧根本看不到丢包** ——
§8.1 的双发 + NACK 把人造丢包补得一个洞不剩，`loss_pct = 0`，而「丢包 > 1% 持续 3 s」因此不成立。
这是**正确行为**（修好了就不该降码率），不是缺陷。要触发降级，注入档要到 **10%** 量级
（接收侧残差约 2%，刚越过 1% 门槛）；注入越猛残差越大（20% → 4~7%，30% → 7~26%），
目标一路降到 §8 下限。报告：`target/evidence/soak/adaptive-sweep-loss{2,10,20,30,50}.json`。

### 13.4 降级分支的端到端回归（新增，稳定复现）

`core/crates/audiolink-engine/tests/engine/adaptive_bitrate_loss.rs`：真实双 Engine + 真实 QUIC +
**开关式高比例丢包中继**（打开时「发送端 → 接收端」每 4 个包只放行 1 个；**反方向照常转发** ——
自适应读的是对端 1 Hz 的 `STREAM_STATS`，那条走的就是反方向）。
从 §8 **上限** 320 kbps 起步，于是第一次变更必然是降级：

```text
注入期间接收侧峰值丢包 6111（×100，缓解后 0）· 中继丢 866 / 转 2602
目标码率时间线 [(320000, 256000, "丢包 61.11% 持续 3 s → 降级"),
                (256000, 204800, "丢包 54.54% 持续 3 s → 降级"),
                (204800, 163840, "丢包 58.33% 持续 3 s → 降级"),
                (163840, 180224, "连续 10 s 无丢包 → 恢复一级")]
```

断言三条：① 注入期间接收侧丢包率 ≫ 1%；② 第一次变更 = 上限 −20%；③ 恢复事件**在降级之后**，
且从降级后的目标 +10% 起步。**连跑 3 次结果一致**（峰值丢包 50~61%、四步时间线完全相同），
单次约 24 s，**不依赖机器负载**（触发条件是丢包率，注入后是几十个百分点）。

**一个反直觉点（本轮踩过）**：**不能**用「整体断流」制造降级 ——
接收侧的丢包率是「收到新序号时发现的洞」算出来的，一个音频包都收不到时**没有洞可算**，
自适应看到的仍然是 0，降级依旧不发生。要的是「高比例丢，但仍收得到一部分」。

### 13.5 遗留

- **桌面 UI**（`desktop/`，不在本轮 write scope）：仍然只把 `CodecAdapted` 写进日志，
  界面上看不到目标码率 —— 这条时间线该接到遥测面板（M2 的另一项交付物）上。
- `docs/23-m2-adaptive-bitrate.md` 里「降级在回环不可复现」那句已经过时（§13.4 给了端到端证据），
  但该文件不在本轮 write scope，留给持有它的人改。
---

## 14. CI 长跑门禁：跑哪个档、为什么（2026-09-17 第 92 轮补）

### 14.1 现状：核心验收在 CI 上原本零门禁

`.github/workflows/ci.yml` 原来只有 nextest（单元 + 集成测试），**没有任何 soak 步骤** ——
「干净链路零容忍」与「8 h 无故障」这两条 M2 验收，CI 上从来没被检查过。
本轮补了一个 **`soak-gate` job：弱网档 + 120 s**。

### 14.2 为什么**不**接严格档（本轮的核心判断，附硬证据）

严格档的判据是「回环稳态不该出现的东西全零」，而 `late_drop` / `underrun` 量的是
**本机调度与回环争用**，不是链路质量。同参数、同代码的实测差异：

| 机器状态 | 观测 | 出处 |
|---|---|---|
| 安静 | 900 s 干净回环严格档 = **`ok`（八类全 0）** | `strict-clean-900s.json` |
| 有负载（同时构建/测试） | 同参数 = **`failed`**（`underrun` 29 / `late_drop` 28） | `strict-clean-900s-autobps.json` |
| 有负载（另一时段） | = **`failed`**（`underrun` 14 / 真迟到 8） | 第 90 轮复核记录 |
| 同一条命令，只看静音占比（60 s 弱网档） | 安静 **0.20%** / 有负载 **9.33%** | §11.6 |

而且争用一旦触发，接收侧抖动深度控制器会进入 ~30.5 s 周期的「降档丢帧」节律（§11.3 末段），
`late_drop` 就按固定节拍长出来 —— **CI runner 上不可控**。
把这类指标接成门禁 = **随机变红**，比没有门禁更糟：团队会开始忽略它。

### 14.3 CI 跑什么：弱网档 + 120 s，判「会话不断 + 掩盖帧 ≤ 1%」

```text
cargo run -q -p audiolink-tools --bin soak-runner -- run \
  --seconds 120 \
  --netem-loss-pct 2 --netem-delay-ms 15 --netem-jitter-ms 15 --netem-bandwidth-kbps 5000 \
  --netem-seed 1 \
  --tolerant --max-underrun-pct 0 \
  --report target/evidence/soak/ci-weak-120s.json
```

三个参数各有理由：

- `--netem-*` 就是 M2 弱网验收的口径（§12：「30 ms 抖动」= ±15 ms，5 Mbps 上限）；
- **`--netem-seed 1`**：注入序列由种子决定（「同种子 = 同一条注入序列」）——
  门禁的**入口**因此可复现，不是随机丢包；
- **`--max-underrun-pct 0`**：静音占比**记录但不判**。它由 `underrun` 派生，对负载敏感
  （上表最后一行：同命令 0.20% / 9.33%），当门禁会变成随机红。字段默认值仍是 **1.00%**（§11.4），
  只有 CI 这一步显式关掉；报告里照旧留下 `underrun_pct_x100` 供人看。

判据就是弱网档原样：**会话不断（`not_streaming` / `stalled`）+ 掩盖帧 ≤ 总帧数的 1%**
（120 s / 20 ms → 6000 帧 → `plc ≤ 60` 帧）。这两个都不随机器负载漂：
会话级判据只在链路真被打断时才涨；PLC 主要由注入丢包决定，且是**比例**口径。

### 14.4 成本（CI 时长是稀缺资源，账要记清）

| 项 | 估算 | 依据 |
|---|---|---|
| checkout / toolchain / rust-cache | ~70 s | `docs/26` §1 实测（6 / 10 / 53 s） |
| `cargo build --bin soak-runner`（cache 命中）| 30–90 s | 依赖都在 cache 里；本地热缓存 4–8 s |
| 门禁本身（120 s 计划）| ~125 s | 观测 120 s + 启停 ~5 s |
| **`soak-gate` job 合计** | **≈ 225–285 s** | 与现有关键路径 `core-heavy`（≈267 s）**同量级** |

它是**独立 job**（与 core-light / core-heavy / android / desktop 并行），进的是
「墙钟 = max(各 job)」这本账 —— 因此**稳态下基本不抬墙钟**（详见 `docs/26` §11）。
**一次性代价**：`rust-cache` 的 key 含 job 名，新 job 首轮 cache 是冷的，预计多 2–4 分钟编译，次轮起消失。
**没做**：没接 nightly 的 8 h；300 s 档位会把关键路径抬到 ~320 s（约 +1 分钟/PR），等真需要再谈 ——
CI 的 120 s 覆盖「档位参数接得对、会话在弱网下稳、判据链路通」，**不**覆盖长时退化（内存 / 漂移 / 慢增长）。

### 14.5 退出码：1 与 2 都必须让 job 红

`soak-runner` 的退出码语义：**0 = 无异常；1 = 有异常；2 = 用法 / 初始化失败**。
CI 步骤里先把它存进 `$code`（后面任何 cmdlet 都会把 `$LASTEXITCODE` 覆盖成 0），
**1 与 2 都直接 `exit` 让 job 红**，顺序是：
有违规先按 **1** 透传（信息量最大）→ 报告缺失 → 红 → **判定 `ok` 但采样数 < 100**（120 s 计划约 117 次）**→ 红（退出码 3）**。
最后那条兜底防的是「这一步根本没真跑，却因为空报告 / 零采样看起来通过」——
它**只对 `ok` 生效**，于是「有违规」永远拿得到 1 这个码（而不是被兜底顶成别的）。
报告以 artifact 上传（`soak-ci-weak-120s`，`if: always()` 保证失败时也留证）。

### 14.6 本地等价验证（第 92 轮实测）

CI 改不了本地真跑，因此按**同一命令**本地跑一遍，并用两条负例钉住退出码语义：

| 场景 | 执行对象 | verdict | 退出码 |
|---|---|---|---|
| 门禁正例 | **`ci.yml` 里那段脚本原文**（120 s，一字未改：从 YAML 里抽出来跑） | `ok`（samples **118**、violations **0**、plc **0**、静音占比 0.15%）| **0** |
| 门禁正例（另一次，同参数） | 同一命令 | `ok`（samples **118**、violations **0**、plc **0**、静音占比 0.28%）| **0** |
| 判据越线 | 同一脚本只改两处：时长缩到 20 s、预算改 `0.01%`（0 拍） | `failed`（`underrun×2`）| **1** |
| 用法错误 | `run --seconds 5 --max-underrun-pct 5`（严格档不接受静音预算）| 不跑，stderr 给出说明 | **2** |

正例那次是**在同一台机器还跑着 8 h 长跑的情况下**做的（负载噪声在），判定仍是 `ok`、plc 0 ——
这正说明「弱网档 + PLC ≤ 1%」这条门禁**不靠机器安静**，与 §14.2 的严格档形成对照。

CI 侧**只做过静态检查**：YAML 用解析器验过（jobs = core-light / core-heavy / **soak-gate** / android /
desktop / version-consistency，步骤与脚本逐字复核），**未**在真实 runner 上执行
（本轮不 push、不触发 CI，也不烧 CI 时间）。首轮跑完请把 `soak-gate` 的实际分步时长补进 `docs/26` §11。





