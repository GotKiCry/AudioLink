# M5 · 依赖许可审计（合规清单的第一块）

> 日期 **2026-09-16**。一句话：发布前必须回答「这些依赖允许我这样分发吗」，
> 而这轮把它做成了**可重复跑 + 有 CI 护栏**的东西，而不是一次性的人工核查。

---

## 1. 为什么分两半

| 一半 | 位置 | 为什么在那里 |
|---|---|---|
| **采集** | `tools/license-audit.ps1` | 要会调 `cargo metadata` 与 `pnpm licenses` —— 这是**环境知识**（工具怎么装、输出什么形状） |
| **判定 + 渲染** | `audiolink-tools::license`（Rust） | 判定逻辑必须能**离线单测**：13 项单测覆盖 OR/AND/WITH、旧式 `/`、括号、未知许可以及报告幂等 |

CLI：`cargo run -q -p audiolink-tools --bin license-audit -- --cargo <json> [--npm <json>] [--write <report.md>]`。
退出码 0 = 无禁止许可；1 = 有（发布前必须处理）；2 = 用法/解析错误。

## 2. 判定语义（SPDX）

| 连接词 | 语义 | 处置 |
|---|---|---|
| `A OR B` | 任一可用即可（**选择权在我们**） | 取最宽松的一项 |
| `A AND B` | 义务叠加，全部必须可用 | 取最严格的一项 |
| `A WITH B` | 例外会改写义务 | 把 `X WITH Y` 当**一个原子**（白名单里显式列出） |

三条**刻意的保守规则**：

1. **未知许可进 `notice`，不进 `denied`** —— 未知不等于安全，也不等于不能用；当允许会漏，当禁止会误杀。
2. **`AND` 与 `OR` 混用的表达式一律进 `notice`** —— 完整 SPDX 优先级（括号/优先级）不在本审计的范围里，
   与其猜，不如交给人看。实测里 `unicode-ident` 的 `(MIT OR Apache-2.0) AND Unicode-3.0` 就是这一档。
3. 旧式写法 `MIT/Apache-2.0`（用 `/` 表示 OR）按历史约定当 `OR` —— 数据里真实存在。

## 3. 实测（首次运行）

| 来源 | 包数 | allowed | notice | denied |
|---|---|---|---|---|
| Rust（`cargo metadata`，含本 workspace） | 635 | 620 | 15 | **0** |
| 前端（`pnpm licenses`） | 88 | 86 | 2 | **0** |

报告落盘在 `docs/compliance/license-report.md`（**自动生成、幂等**：不含时间戳与路径，CI 里跑不会产生 diff）。

### 3.1 报告暴露出的真实待办（这就是这块工作的价值）

| 发现 | 影响 |
|---|---|
| **`uniffi` 全家 8 个包是 MPL-2.0** | 文件级 copyleft：可以静态链接进闭源产品，但**被修改过的 MPL 文件必须公开**。我们没改过 uniffi 源码 → 当前合规，但这条要写进发布检查单 |
| `lightningcss`（前端构建工具链）2 个包 MPL-2.0 | 只进构建，不进分发产物 → 义务更轻 |
| `webpki-root-certs` 是 `CDLA-Permissive-2.0` | 宽松许可但不在白名单 → 进 `notice`（**没有自动放行，这是对的**） |
| `unicode-ident` 的 `(MIT OR Apache-2.0) AND Unicode-3.0` | 混用表达式 → `notice`；实际两项都宽松，但审计不做优先级推断 |

**没有 GPL / AGPL / SSPL / 非商业许可** —— 这是「可以发布」的前提结论，而且它是**可重复验证**的，不是一次性的。

## 4. CI 护栏

`desktop` job 新增一步（那里同时有 `cargo` 与 `pnpm`，两边都能采到）：

```yaml
      - name: License audit (Rust + frontend)
        run: pwsh tools/license-audit.ps1
```

意义：**新引入一个不能分发的依赖会在 CI 里红**，而不是等到发布前才发现。

## 5. 覆盖面（诚实清单）

- **已覆盖**：Rust workspace 的全部依赖、桌面前端依赖；
- **未覆盖**：Android（Gradle）依赖、随包分发的二进制内部第三方库、字体与图标资源；
- 本报告回答的是「许可是否允许这样分发」；**署名/免责文本的实际投放位置是另一件事**（还没做）。
