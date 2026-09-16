# M2 · soak-runner（回环长跑 + 指标采集 + 异常快照）

> 看板：`[工具] soak-runner（8 h 回环 + 指标采集 + 异常快照）`。日期 **2026-09-16**。
> 一句话：把「跑 8 h 不出故障」从「人盯着看日志」变成**可判定的产物** ——
> 逐秒采样 → 越界留快照 → JSON 报告 → **退出码即判定**。

---

## 1. 用法

```text
soak-runner run [--seconds 28800] [--frame-ms 20] [--report PATH] [--expected-bps 320000] [--warmup-seconds 3] [--quiet]
                [--tolerant] [--netem-loss-pct N] [--netem-delay-ms N] [--netem-jitter-ms N] [--netem-bandwidth-kbps N] [--netem-seed N]
```

| 参数 | 含义 |
|---|---|
| `--seconds` | 观测时长，默认 28800（8 h）；CI 冒烟用 60 |
| `--frame-ms` | 帧长（10 / 20 / 40 / 60），默认 20 |
| `--report` | 报告路径，默认 `target/evidence/soak/soak-<unix>.json` |
| `--expected-bps` | 目标码率，默认 320000（冗余双发后的期望）；`0` = 不判码率 |
| `--warmup-seconds` | 预热秒数，默认 3（预热期的越界不判） |
| `--quiet` | 不打印每秒进度 |
| `--tolerant` | 弱网档判据：只钉「会话不断 + 掩盖比例 ≤ 1%」，不判欠载/迟到/NACK/瞬时丢包（详见 `docs/24`） |
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
| `underrun` | 播放欠载计数增加 |
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
    "final_stats": { "bitrate_bps": 320800, "loss_pct_x100": 0, "underruns": 0,
                      "plc_count": 0, "late_drops": 0, "nack_count": 0,
                      "buffer_level_us": 20000, "rtt_us": 1848, "...": 0 }
  },
  "violations": [],
  "coarse": [ { "bucket": 0, "at_secs": 3, "stats": { "...": 0 } } ]
}
```

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
| 单元（8 项） | 稳态零异常、计数器增量 → 快照（含 delta 文案）、丢包/码率/挂住判定、状态离开 streaming 只记一条、快照上限与 `dropped_violations`、每分钟粗采样、JSON 报告字段、摘要文本 |
| 端到端（人工，本条给出命令与数字） | 20 s 冒烟（exit 0）+ 目标码率写错（exit 1） |

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


