# AGENTS.md — core(Rust 内核)

workspace 的全部跨平台内核 crate;各 crate 职责表与依赖方向见根 `AGENTS.md` 布局地图,本文件只补内核特有的导航与坑。`core/src/` 与 `core/benches/` 是空占位,别在里面放东西。

## 编排层(audiolink-engine)内部地图

绝大多数内核改动发生在这里。`src/runtime.rs`(约 5.6k 行)是装配与线程本体,`src/runtime/` 是它的子模块:

| 路径 | 是什么 |
|---|---|
| `runtime.rs` | 会话循环、采集枢纽、编码线程、播放线程、看门狗装配 |
| `runtime/jitter.rs` | 抖动深度控制器 + 播放深度状态机(含稳态水位结算)+ 序号重排窗 |
| `runtime/backlog.rs` | 全链积压回收(仅单源、非预约、有平台反馈) |
| `runtime/nack.rs` | NACK 追踪与重传缓冲 |
| `runtime/sink_watchdog.rs` | FR-28:2 s 无输出重建 sink |
| `runtime/playout_tests.rs` | 播放循环的 `#[cfg(test)]` 场景测试 |
| `epoch.rs` / `clock.rs` | §7 预约播放排程 / 时钟探测节拍与发送 |
| `telemetry.rs` / `silence.rs` | 遥测聚合 / 连续静音记账(均为进程内口径,见下) |

## 测试布局

- 单元测试在各 src 文件的 `#[cfg(test)] mod tests`;
- 集成测试在 `core/crates/audiolink-engine/tests/engine/`(按主题一文件,入口 `main.rs`);
- `tests/water_level_repro.rs` 是 `#[ignore]` 的手动复现装置(设 `WATER_SECS` 控制时长),不进 CI。

## 内核硬规则

- **wire schema 冻结**:`StreamStats` 与控制帧走 postcard 严格解码,尾随字节即非法 —— 不许追加字段。进程内可见计数走 `Engine` 的独立出口,先例:`depth_drops`(`telemetry.rs` 有完整注释说明为什么)。
- proto 编解码改动必须同步 `audiolink-proto/tests/golden_vectors.rs`(跨端契约,见根 `AGENTS.md` 动手前)。
- 水位/欠载治理有两版护栏被真机否决的历史(docs/12 §11.12–§11.14):动这片逻辑前先读 docs/12 §11 与 docs/69;任何回收/结算必须速率受限,且保留目标之上一整帧抖动摆幅。
- 自检快路径:`cargo xt -p <crate>`;提交前仍须跑根 `AGENTS.md` 门禁全套。
