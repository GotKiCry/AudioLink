# M2 · 弱网测试台（netem-sim）与弱网长跑验收

> 日期 **2026-09-16**。一句话：把路线图 M2 的验收条件「弱网（**5 Mbps / 2% 丢包 / 30 ms 抖动**）可听」
> 变成**可执行的工具 + 可判定的产物**：`netem-sim` 注入，`soak-runner --netem-* --tolerant` 长跑判定。

---

## 1. 交付物

| 组件 | 位置 | 作用 |
|---|---|---|
| `audiolink-tools::netem` | `core/crates/audiolink-tools/src/netem.rs` | 注入内核（丢包 / 延迟 / 抖动 / 限速）+ 透明 UDP 中继，7 项单测 |
| `netem-sim` | `--bin netem-sim` | 独立工具：把发起方连到中继即可（路线图 M2 交付物 6） |
| `soak-runner --netem-*` | `--bin soak-runner` | 一条命令跑「弱网 + 长跑 + 判定」 |
| `soak-runner --tolerant` | 同上 | 弱网档判据（见 §3） |

### 1.1 注入内核的三条纪律

1. **确定性**：随机数来自自带 xorshift64*，同种子 = 同一条注入序列 —— 弱网验收必须可复现，
   否则「这次卡顿是不是网络造成的」永远说不清；
2. **纯逻辑**：只回答「这个包该丢还是该延迟多少」，不碰套接字（收发在中继/工具里）；
3. **只丢不改**：不重排、不改写字节；乱序由「延迟抖动的随机性」自然产生。

---

## 2. 用法

### 2.1 独立工具（把发起方连到中继）

```text
netem-sim run --upstream 127.0.0.1:58290 [--loss-pct 2] [--delay-ms 15] [--jitter-ms 15] \
              [--bandwidth-kbps 5000] [--seed 1] [--seconds 60] [--report PATH] [--quiet]
```

工具会打印自己的监听地址；把**发起方**（`device-link` / `link-loop` / 桌面端手工连接）的连接地址
改成它即可。QUIC 是端到端加密的，所以中继只搬字节、不需要理解协议。

### 2.2 一条命令跑 M2 弱网验收

```text
cargo run -q -p audiolink-tools --bin soak-runner -- run --seconds 60 --tolerant \
  --netem-loss-pct 2 --netem-delay-ms 15 --netem-jitter-ms 15 --netem-bandwidth-kbps 5000 \
  --expected-bps 0 --report target/evidence/netem/soak-weak.json
```

给了任一 `--netem-*` 参数时，`soak-runner` 会在两个 Engine 之间插一个弱网中继（node-a 连中继、
中继转给 node-b），因此**长跑与注入是同一条命令**。

---

## 3. 弱网档判据（`--tolerant`）

回环稳态档是**零容忍**的（任何非零欠载 / 掩盖 / 丢包都是缺陷信号）；弱网下这套判据必然全红 ——
欠载与迟到本来就是给定条件的一部分。于是弱网档只钉两件事：

| 判据 | 值 | 为什么 |
|---|---|---|
| 会话不断 | 全程 `streaming` | 断流是硬故障 |
| 掩盖比例 | `plc_count ≤ 总帧数 × 1%` | 这是「**可听**」的量化版本：偶发掩盖可以，成片掩盖不行 |
| 欠载 / 迟到 / NACK | 不判 | 弱网给定条件的一部分 |
| 瞬时 `loss_pct_x100` | 不判 | 它是 1 Hz 窗口值，实测在 2% 注入下会瞬间跳到 2%～4%，而当窗口的平均掩盖只有 0.13% —— 拿瞬时值当门槛只会制造假警报 |
| 瞬时码率 | 不判 | **8 h 长跑补上的判据**：弱网下瞬时码率随自适应与重传从 205 kbps 摆到 427 kbps（目标 320 kbps），硬阈值在弱网档只会制造假警报 |

（实现上这也补了一个真实缺陷：`SoakThresholds` 里的 `max_underruns` / `max_plc` / `max_late_drops` /
`max_nack` 四个字段此前**定义了却没被判定逻辑使用**；现在 `check_delta` 真的按「累计值超过阈值的增量」判，
默认值 0 时行为与之前完全一致。）

> **8 h 实测的教训（2026-09-17）**：这份清单最初漏了码率判据，于是 8 h / 5 Mbps / 2% 丢包 / 15±15 ms 的一次真实长跑
> 判出 7055 条异常，**全部**是 `bitrate_out_of_range`（集中在 2692..8293 s）。现在档位集中在
> `SoakThresholds::weak_network(planned_secs, frame_ms)`，码率以 `bitrate_tolerance_pct_x100 = 0`（= 不判）关掉，
> 并有单测钉住「0 就是不判」。同参数 90 s 对照：`--tolerant` → 0 异常 / exit 0；去掉 `--tolerant` → 18 条异常 / exit 1。
> 同批修掉的另两条（快照配额按类、`violations_by_kind` 等报告字段）见 `docs/22-m2-soak-runner.md` §9。

---

## 4. 实测

### 4.1 注入内核自账（60 s，2% 丢包 / 15±15 ms / 5 Mbps）

```json
{"stats": {"observed": 5676, "forwarded": 5571, "dropped_loss": 105, "dropped_bandwidth": 0,
           "observed_loss_pct_x100": 185}}
```

实测丢包 1.85%（配置 2.00%），限速丢包 0 —— 5 Mbps 对 320 kbps 的链路绰绰有余。

### 4.2 弱网长跑（M2 验收口径，60 s）

| 观测项 | 实测 | 说明 |
|---|---|---|
| 会话状态 | 全程 `streaming` | 60 s 无断流 |
| 接收侧丢包率 | **0** | 2% 注入被「冗余双发 + NACK」完全修复 —— 这正是 §8.1 的设计意图 |
| PCM 掩盖 | **4 帧 / 约 3000 帧 ≈ 0.13%** | 远低于 1% 的可听门槛 |
| 欠载 / 迟到 | 10 / 12 | 弱网给定条件的账，不判 |
| NACK 重传 | 0～3 | 大多数洞被副本先补上，重传只是兜底 |
| RTT（平滑） | ≈ 41 ms | 15 ms 单向延迟 + 抖动 + 队列 |
| 判定 | **ok（exit 0）** | `--tolerant` 档 |

### 4.3 一个值得记下来的结论：弱网下**不该**降码率

上一轮（自适应码率）留下一个悬案：回环上注入 40% 丢包也触发不了降级，因为双发 + NACK 把洞全补上了。
本轮用 **2% 注入**（真实弱网档位）复现了同一个结论，而且这次是**正确的行为**：

- 接收侧上报的丢包率是 **0**（所有洞都被修复）；
- 于是 §8 的「丢包 > 1% 持续 3 s」条件不成立 → 码率**保持 320 kbps 不降**；
- 掩盖比例只有 0.13% → 听感事实上是好的。

换句话说：**「链路被修好了」和「链路没坏」在自适应码率眼里是同一件事**（这没错），
真正会触发降级的场景是「修复能力本身不够」的链路 —— 那需要更高的丢包率或更长的 RTT，
属 M2 弱网验收的后续档位（例如 10% 丢包 / 100 ms RTT）。

---

## 5. 验证清单

| 层 | 内容 |
|---|---|
| 单元（7 项） | 透传零延迟、同种子复现、2% 配置 → 实测 1.5%~2.5%、延迟落在基准 ± 抖动内且真的散开、令牌桶限速后的转发吞吐不超过上限两倍、报告 JSON 字段、默认档与 M2 口径一致 |
| 单元（soak 9 项） | 含「阈值内的增量不记异常 / 超过阈值的增量按差额记账」 |
| 端到端（人工，本条给出命令与数字） | 60 s M2 弱网口径长跑：exit 0、掩盖 0.13%、全程 streaming |

```
cargo test -p audiolink-tools --lib netem
cargo test -p audiolink-tools --lib soak
cargo run -q -p audiolink-tools --bin soak-runner -- run --seconds 60 --tolerant \
  --netem-loss-pct 2 --netem-delay-ms 15 --netem-jitter-ms 15 --netem-bandwidth-kbps 5000 --expected-bps 0
```

---

## 6. 未做 / 后续

| 项 | 说明 |
|---|---|
| 更高丢包档（10% / 100 ms RTT） | 用来触发**自适应降级**的端到端验证（见 §4.3） |
| 真机弱网 | 本轮注入在回环上；PC → Android 的弱网验收需要真机（M1/M2 真机项） |
| 8 h 弱网长跑 | **已执行**（28800 s，2% 丢包 / 15±15 ms / 5 Mbps）：暴露并修掉三个判定缺陷（见 §3）；掩盖率 0.066%、注入自账 2.01%、末次码率 349 kbps / RTT 41.8 ms / 漂移 −4 ppm。修完判据后**需要重跑一次**才谈得上「8 h 通过」 |
| 乱序注入（显式） | 目前靠延迟抖动的自然乱序；显式的「固定乱序率」如果需要再加 |

