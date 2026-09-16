# M2 · 遥测面板（曲线 + 日志导出）

> 日期 **2026-09-16**。一句话：把桌面端的遥测从「十个数字」升级成「数字 + 5 分钟曲线 + CSV 导出」，
> 补上 M2 交付物清单里的「遥测：`StreamStats` 全字段采集 + 桌面端遥测面板（曲线 + P50/P95）+ 日志导出」。

---

## 1. 交付物

| 层 | 内容 | 位置 |
|---|---|---|
| 契约类型 | `TelemetryRow`（导出用的扁平行，camelCase） | `desktop/src-tauri/src/view.rs` |
| 渲染 | `render_telemetry_csv`（**纯函数**：表头 + 一行一个采样点） | 同上 |
| 落盘 | `write_telemetry_csv`（建目录、按毫秒时间戳命名、写文件） | `desktop/src-tauri/src/engine_bridge.rs` |
| 命令 | `export_telemetry(rows) -> 落盘路径` | `desktop/src-tauri/src/lib.rs` |
| 前端 | `TelemetryRow` 类型 + `telemetryRowOf` + `api.exportTelemetry` | `desktop/src/types.ts`、`lib/ipc.ts` |
| 历史 | 500 ms 一个采样点、上限 **600**（5 分钟）的环形历史 | `desktop/src/lib/useAudioLink.ts` |
| UI | 4 条 SVG 折线（端到端 / 缓冲水位 / 码率 / 丢包）+ 导出按钮 | `desktop/src/components/TelemetryPanel.tsx` |

P50 / P95 早在 M1 就在 `TelemetryView` 里（数字面板已有），本轮不再重复造。

---

## 2. 设计取舍

### 2.1 曲线为什么手绘 SVG 而不引图表库

四条折线不值得多一个运行时依赖；而且手绘能自己钉住两个细节：

- **纵轴按本窗口最大值归一化**：遥测关心的是「趋势与尖峰」，不是绝对刻度，所以不画网格与坐标轴，
  精确值在数字面板里；
- `vectorEffect="non-scaling-stroke"` + `preserveAspectRatio="none"`：线宽不随容器缩放变形，
  刷新时也不会重排布局（UI 规格 §5 的同一条要求）。

### 2.2 为什么导出走「前端累积 → 后端写文件」

- 历史在**前端**（它本来就是采样点的消费者），导出只是把它交给一个能落盘的地方；
- 后端**不弹文件对话框**（那要引入 dialog 插件，且无头/CI 下不可用）：目录固定在 `<用户目录>\AudioLink`，
  路径由命令返回、UI 原样显示；
- 导出的行形状与界面快照**严格同形**（`TelemetryRow` = `TelemetryView` + `atUnixMs`），
  两侧各有一份镜像测试：Rust 侧 `from_view`（`#[cfg(test)]`）漏一个字段编译期就红，
  TS 侧 `telemetryRowOf` 用展开运算符同理。

### 2.3 CSV 是持久化契约，不是界面字段的投影

`TelemetryRow` 特意**不复用** `TelemetryView`：列名一旦写进用户的文件就不该再跟着界面字段漂。
所以导出 CSV 有自己固定的列名与顺序（`docs` 与测试都钉住），界面字段改名时导出不受影响。

---

## 3. 验证

| 层 | 内容 |
|---|---|
| Rust 单测（view.rs） | CSV 表头 + 一行一个采样点 + 小数位固定 + 以换行收尾；空历史只有表头；前端 JSON 形状能反序列化（字段名漂了就红）；`TelemetryRow` 序列化形状与 TS 类型逐字段一致 |
| Rust 单测（engine_bridge.rs） | `write_telemetry_csv` 真的落盘：目录被创建、文件名带 `telemetry-` 前缀、读回内容首行是表头且行数 = 采样点数 + 1 |
| 前端 | `tsc --noEmit`（严格模式，含 `noUncheckedIndexedAccess`）+ `vite build` 通过 |
| 质量门 | 内核 workspace 与桌面 crate 的 fmt / clippy `-D warnings` / 全部测试 |

```
cargo test -p audiolink-desktop
cd desktop; pnpm build
```

> 未做的验证：**真实窗口里的曲线观感**与「点了导出按钮真的写到我机器上」这两件事需要跑起 Tauri 应用
> 并有人看（本机是无头环境）。曲线代码的编译期约束与导出落盘的单测覆盖了逻辑面，
> 视觉面留给真机/人工复核 —— 与桌面端此前的 UI 项同样的口径（`docs/13`）。

---

## 4. 未做 / 后续

| 项 | 说明 |
|---|---|
| 托盘 / 自启 / 双语 | M5 范围（路线图 M5） |
| 曲线导出为图片 | 需要时再加（CSV 已经够做离线分析） |
| 更长历史的滚动归档 | 目前上限 5 分钟，更长看导出的 CSV |

