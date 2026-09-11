# AudioLink 总览（一页纸）

> 本文是入口文档：先读它，再按需深入。最后更新：**2026-09-11**（需求对齐完成，等待开发）

---

## 一句话

**AudioLink 是一个局域网实时音频分发系统：任意节点都能把声音低延迟、可同步、可混合地推给任意其他节点。**

## 关键数字

| 指标 | 目标 |
|---|---|
| 端到端延迟 | **P50 80–110 ms（Wi-Fi 现实档）/ 40–55 ms（有线·理想档）**，P95 ≤ 150 ms |
| 设备间同步 | 设计目标 **≤5 ms**，验收线 **≤10 ms（P95）** |
| 默认码率 | Opus 48 kHz / 20 ms / **160 kbps**（自适应 96–320 kbps）；可选 PCM 无损档 1.536 Mbps |
| 并发 | 单发送端 ≥ 8 个接收端；单接收端 ≥ 4 路混音 |
| 首帧 | ≤ 300 ms |
| 长时稳定 | 8 h 无崩溃、无静音、无累积漂移 |
| 平台 | Windows 10 1809+ / 11；Android 8.0+（API 26+） |
| 安全 | QUIC TLS1.3 + 自签证书指纹（TOFU）+ 6 位 PIN 配对白名单 |

## 边界（用户已确认）

**做**：双向网状互推、多设备并发、声道分流、多房间同步（临时同步组）、Wi-Fi 自动发现、托盘/自启/音量联动、遥测与日志导出、手机系统内录（Android 10+）。

**不做**：云音乐（musiche）、斐讯 R1 灯效、USB/adb 通道、远程 Web 管理页、跨公网穿透、iOS/Web 端、Android 开机自启。

## 技术骨架

```
Rust 内核（core/crates/*）
  ├─ 协议 ALP/2 · Opus · 抖动缓冲 · 时钟同步 · 混音 · 发现 · 配对
  ├─ 桌面端：Tauri 2 外壳 + React 19 + Tailwind（进程内直接调用）
  └─ Android：Kotlin + Compose UI + AudioTrack/AudioPlaybackCapture（经 UniFFI/JNI 调用同一内核）
传输：QUIC（quinn）—— 音频走数据报，控制走可靠流
```

## 决策速查（谁定的）

| 决策 | 结论 | 出处 |
|---|---|---|
| 拓扑 | **完全网状**：任意节点互推（含 Android → Android） | 用户 R3 |
| 传输 | **QUIC**（用户裁决；两条调研均建议裸 UDP，已记录理由与回退路径） | ADR-004 |
| 内核 | Rust 写一次，两端共用（Android 走 JNI/UniFFI） | 用户 R4 |
| 编码 | `opus-rs` 纯 Rust（零 C 依赖）+ `RESTRICTED_LOWDELAY` + 锁 48 kHz | ADR-003 / §8 |
| 丢包对抗 | **NACK 重传 + PLC 为主**；音乐场景**关闭** in-band FEC | §8.1 |
| 桌面端 | Tauri 2 + React 19 + TS + Tailwind 4 | 用户 R3 |
| Android | Kotlin + Compose（完全重写），minSdk **26** | 用户 R3/R7 |
| 同步 | 四时间戳时钟同步（200 样本中位数）+ 预约播放 | 用户 R4 |
| 配对 | 自签证书 + TOFU 指纹 + PIN 白名单 | 用户 R3 |
| 许可 | Apache-2.0，保留上游归属（NOTICE） | 用户 R4 |
| 起点 | 先打穿单链路（PC → 单手机） | 用户 R4 |
| 版本红线 | `compileSdk 37` / **`targetSdk` 必须停在 36** / JDK 17 | ADR-009 |
| 蓝牙 | 物理限制（100–300 ms），UI 提示，不作为达标场景 | 用户 R7 |

## 文档地图

| 想了解 | 读 |
|---|---|
| 需求、验收标准、决策追溯 | [`01-requirements.md`](01-requirements.md) |
| 架构分层、线程模型、同步与混音 | [`02-architecture.md`](02-architecture.md) |
| 协议字节级规格（可照着实现） | [`03-protocol.md`](03-protocol.md) |
| 为什么选这些技术、版本矩阵、回退路径 | [`04-tech-stack.md`](04-tech-stack.md) |
| 先做什么、怎么验收 | [`05-roadmap.md`](05-roadmap.md) |
| 环境怎么装、坑在哪 | [`06-dev-environment.md`](06-dev-environment.md) |
| 从 AudioPipe 继承/摒弃了什么、合规红线 | [`07-migration-notes.md`](07-migration-notes.md) |
| 界面长什么样 | [`08-ui-spec.md`](08-ui-spec.md) |
| 对标 Snapcast/Jamulus 等学到了什么 | [`09-benchmark-notes.md`](09-benchmark-notes.md) |

## 现在就能做的三件事（M0 起步）

1. **跑通空壳构建**：`cargo test` + `gradlew assembleDebug` + `pnpm tauri dev` 三条链路各跑一次（顺带验证 AGP 9 + 内置 Kotlin + Compose 插件组合是否可用；不可用则切路线 B：AGP 8.13.2 + compileSdk 36 + BOM 2026.06.01）。
2. **实现 `audiolink-proto`**：按 `03-protocol.md` 写帧编解码并让 §12 的 golden vectors 全绿 —— 这是所有后续工作的地基。
3. **实测两条硬指标**：用 `tools/latency-probe` 量出采集/编码/网络/缓冲/播放五段延迟分解，确认"零重采样 + LowLatency"两条硬性验收用例（NFR-13/14）当前可达。
