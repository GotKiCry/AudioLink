# 护栏与工具补齐（定长载荷自洽 / 文档脱敏 / alp2-dump 抓包）

> 对应看板三项：`[护栏] 「定长载荷自洽」测试：LEN == 字段宽度之和`、`[安全] docs/06-dev-environment.md 本机路径脱敏`、
> `[工具] alp2-dump 支持 pcap/pcapng 输入`。日期 **2026-09-16**。
> 三项都不触碰协议语义与实时音频路径：一项是机械护栏，一项是仓库卫生，一项是调试基础设施。

---

## 1. 定长载荷自洽护栏

### 1.1 要防的缺陷

ALP/2 的定长载荷（`CLOCK_PROBE` 12 B、`CLOCK_REPLY` 28 B、`KEEPALIVE` 0 B、`NACK` 4×n B）此前在**四处各写一份账本**：
§3 载荷表（文档）、`Ptype::payload_len_rule()` 的字面量、`ClockProbe::LEN` 这类常量、`encode_into`/`decode` 里的裸偏移数字。
任何一处漂移，症状都是「线上看不懂的拒绝」或「静默错位」。

更麻烦的是：既有回归拿 `X::LEN` 当期望值（**自指**）—— 常量自己算错时，长度扫掠照样全绿。

### 1.2 做法

- `audiolink-types`：新增字段宽度常量（`U8_LEN` / `U16_LEN` / `U32_LEN` / `U64_LEN` / `I64_LEN`）与**字段偏移**常量，
  长度一律写成「最后一个字段的偏移 + 该字段宽度」：
  `CLOCK_PROBE_PAYLOAD_LEN = CLOCK_PROBE_T1_OFFSET + I64_LEN`；
  `DATAGRAM_HEADER_LEN` / `CONTROL_HEADER_LEN` 同样改成宽度之和（值不变：24 / 8）。
- `audiolink-proto`：`Ptype::payload_len_rule()`、`ClockProbe::LEN`、`ClockReply::LEN`、`NACK_MAX_ITEMS`
  全部引用同一批常量；编解码里的裸偏移（0 / 4 / 12 / 20）换成偏移常量；
  定长长度不符的拒绝消息由常量拼出（`... must be exactly {LEN} B, got {n} B`），消息与常量不可能再漂移。
- 新增 `core/crates/audiolink-proto/tests/payload_len_table.rs`：**§3 载荷表的独立副本**
  （帧头 7 字段、CLOCK_PROBE 2 字段、CLOCK_REPLY 4 字段、控制帧头 4 字段的「名字 + 宽度」表，以及表值 24 / 12 / 28 / 8），断言四件事：
  1. 实现常量 == §3 表值；
  2. 实际编码长度 == 字段宽度之和，并按表累加偏移读回哨兵字段；
  3. `Ptype::ALL` 的规则表与编码器一致：Exact 的 len±1 被拒、U32List 的 0 项 / max+1 项 / 非 4 倍数被拒、Opaque 吃到 MTU 上限；
  4. 控制帧头与 ptype 表自洽（6 项、`from_u8`/`as_u8` 互逆、未知值 → `None`）。

### 1.3 破坏性验证（证明护栏真的会响）

| 人为注入的缺陷 | 期望 | 实测 |
|---|---|---|
| `CLOCK_REPLY_PAYLOAD_LEN` 漏掉最后一个 i64（变 20） | 红 | ✅ 3 个用例失败：`Exact { len: 20 }` vs `28`，且 20 B 载荷被接受 |
| `payload_len_rule()` 里写死错误字面量 24 | 红 | ✅ 同样 3 个用例失败 |
| 解码侧 `t2` 用错偏移（编码/解码漂移，长度不变） | 红 | ✅ `clock_reply_len_equals_sum_of_field_widths` 失败（读出 t2 = 1000121，应为 1000120） |

三处破坏均已恢复；恢复后 `cargo test -p audiolink-proto` 全绿（新增 9 项用例、既有 29 项不动）。

---

## 2. 文档本机路径脱敏

仓库已 PUBLIC，`docs/` 下多处写着开发机的绝对路径（含用户名与 scoop 安装位置）。
做法：统一换成占位符，并在 `docs/06-dev-environment.md` §1 顶部给出**占位符对照表**与「怎么查本机实际值」。

| 脱敏前的内容 | 现在写成 | 说明 |
|---|---|---|
| 用户目录下的 Android SDK 绝对路径 | `%LOCALAPPDATA%\Android\Sdk` | Windows 通用环境变量，可直接用 |
| Scoop 安装的 Corretto 17 绝对路径 | `<JDK 17 根目录>` | 本机即 Scoop 装的 Corretto 17 |
| `local.properties` 里的 `sdk.dir` 转义写法 | `<Android SDK 根目录>` | 同上 |
| `local.properties` 里的 `storeFile` | `<你的 keystore 路径>` | keystore 不入库 |
| 其它零散出现 | `<本机用户目录>` | 兜底替换 |

覆盖 7 个文件：`docs/06`、`10`、`11`、`12`、`14`、`15`、`16`。
验证：全仓搜开发机用户名 `liuzh` → **0 命中**（`tools/` 里的示例本就已是 `<你>` 占位符）。
命令示例的可执行性不受影响：占位符连同单引号一起替换即可（`-JavaHome '<JDK 17 根目录>'`）。

---

## 3. alp2-dump 支持 pcap / pcapng

### 3.1 用法

```text
alp2-dump [FILE|-] [--single] [--quiet] [--pcap] [--port N] [--all-ports]
```

- 输入**按内容自动分派**：4 字节 magic 命中 pcap / pcapng → 抓包分支；否则仍按 hex 文本（原行为完全不变）；
- 抓包分支只解 `--port`（默认 **58290**，§1 的 QUIC 端口）上的 UDP 载荷，`--all-ports` 关掉该过滤；
- 「剥不出 UDP 载荷」（ICMP / 别的 UDP 服务 / 未知链路层）计入**跳过**而不是解码失败 ——
  否则一次普通抓包会被无关流量淹没成一片红。

### 3.2 实现边界

新增 `audiolink-tools::pcap`（**不引第三方抓包库**，只按格式规范解析）：

- **pcap**：四种 magic（µs/ns × 大小端）都认，逐记录按 `caplen` 取帧；
- **pcapng**：SHB（用 byte-order magic 决定端序）+ IDB（链路类型）+ EPB + SPB；块长必须 4 字节对齐，
  且**块尾副本必须等于块头**（防把垃圾当块）；
- **链路层 → UDP**：Ethernet（含 802.1Q / 802.1ad 的 VLAN 标签链）、Linux SLL v1、raw IP、BSD loopback；
- **IP**：IPv4（按 IHL）/ IPv6（固定 40 B）；`udp_len < 8`（GSO 抓包常见）按「剩下的全是载荷」处理；
- **不做**：IP 分片重组、IPv6 扩展头链、隧道解封装 —— 一律判「不是目标包」（绝不猜）。

### 3.3 夹具与回归

`core/crates/audiolink-tools/tests/pcap_extract.rs`（9 项）—— 夹具全部**现场合成**，帧头逐字节按 RFC 拼：

1. pcap + Ethernet/IPv4/UDP → 剥出 §12 的 CLOCK_PROBE 向量，再交给 L1 严格解码；
2. pcapng EPB 同链路（链路类型取自 IDB）+ 时间戳；
3. 纳秒 magic 的时间戳换算；
4. 双层 VLAN 标签；
5. IPv6 与 Linux SLL；
6. ICMP / 未知链路类型 / IPv6 扩展头 → 跳过（返回 `None`，不是误解析）；
7. `udp_len = 0` 回退；
8. 空输入 / 坏 magic / 截断全局头 / 记录超长 / 块长非法 / 块尾不一致 → 一律 `Err`，**无 panic**；
9. hex 文本不会被误判成抓包。

端到端（样例抓包由脚本合成后落在 `target/evidence/alp2-dump/`，不入库）：

```text
> cargo run -q -p audiolink-tools --bin alp2-dump -- target/evidence/alp2-dump/sample.pcap
[2] 36 B  udp 58290→48210  → 数据报 CLOCK_PROBE(0x03)  ver=0x02 flags=0x0000 stream_id=0 seq=0 sample_index=0 epoch_id=0x0000000000000000
        CLOCK_PROBE probe_seq=7 t1=1000000 µs
[3] 36 B  udp 58290→48211  → 数据报 CLOCK_PROBE(0x03)  ver=0x02 flags=0x0000 stream_id=0 seq=0 sample_index=0 epoch_id=0x0000000000000000
        CLOCK_PROBE probe_seq=7 t1=1000000 µs

共 2 个 UDP 报文（pcap / 链路类型 1，跳过 2 个非目标包），解码成功 2，失败 0
```

- pcapng 结果相同（`pcapng` 字样 + 同一份解码输出）；
- `--all-ports` 时同抓包里的 DNS 包被计入失败（这正是端口过滤要挡的噪声）；
- hex 文本路径回归通过（§12 两行向量照旧解码）；退出码 0。

---

## 4. 质量门与影响面

| 项 | 结果 |
|---|---|
| `cargo fmt --all --check` | ✅ 干净 |
| `cargo clippy --workspace --exclude audiolink-desktop --all-targets --all-features -- -D warnings` | ✅ 干净 |
| `cargo test --workspace --exclude audiolink-desktop` | ✅ 全绿（proto 新增 9 项、tools 新增 9 项） |
| Android / 桌面 | 本轮**未改动** Kotlin / TS / Tauri 侧，未重跑 |

线上行为无变化：`DATAGRAM_HEADER_LEN` = 24、`CONTROL_HEADER_LEN` = 8、两处定长载荷 12 / 28 B 与 §3 表一致 ——
护栏只是把它们从「四处手抄」变成「一处算式 + 一份独立副本」。
