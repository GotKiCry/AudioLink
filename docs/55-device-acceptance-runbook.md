# 真机验收手册（device acceptance runbook）

> 日期 **2026-09-17** ｜ 事实基线：写这份手册时的 HEAD 工作区（**没有真机在场**：`adb devices` 为空）
> 用途：真机到位后**照着做就能出结论**，不必现场再设计量法。本手册把「方法」这一半做完，把「设备」那一半留给现场。
> 上游：`docs/05-roadmap.md` §M1/§M3/§M4 验收表 · `docs/12-m1-device-acceptance.md`（M1 真机报告）· `docs/16`（PHK110 复测）·
> `docs/27-m3-sync-measure.md`（量具）· `docs/47-m1-latency-guard.md`（回环护栏的边界）· `docs/54-m1-audit.md`（第 113 轮 M1 核对）

## ⚠️ 先读这一页（这份手册的效力边界）

1. **本手册里的每一条步骤都没有在真机上跑过** —— 写它的时候设备不在场（`adb devices` 空）。
   凡是**没有实际验证过**的步骤，就地标 **⚠️ 未经实机验证**；集中在 §7 再列一遍。
   **不要**因为「手册写了」就当成「已验证流程」——本项目已经为「看起来完成了」吃过好几次亏。
2. **可执行的边界**：判读阈值、命令、误差项的量级都写死了；**具体的设备名、距离、音量、环境**必须现场定（见 §2.10 的「必须现场定」清单）。
3. **证据纪律**：`target/evidence/` 是 **gitignored**（`.gitignore` 第 2 行 `/target/`）—— 原始 WAV / 日志 / JSON 都不入库。
   所以**结论必须落文档**：本条验收做完后，把「读数 + 出处 + 日期 + 设备」写回对应文档（M1 → `docs/12` 或新开一节；M3/M4 → 各自 docs），
   并把关键文件**重算 SHA-256 写进文档**（文件会随 `target/` 被清掉，哈希不会）。
4. **一次只改一个变量**：真机测量最贵的资源是「设备在场的时间」，因此每轮都记录：手机型号/系统/包名、PC 端点、距离、房间、音量、**当轮的软件版本（git hash）**。

---

## 0. 目录与可执行度总表

| # | 条目 | 手册章节 | 当前可执行度（核实于本机） |
|---|---|---|---|
| 1 | **[M1] 验收**：声学端到端 P50 ≤ 110 ms / P95 ≤ 150 ms | §2 | **方法齐备、命令齐备，但整条链路未验证**；需要一支麦克风 + 现场标定。零重采样 / 低延迟模式两项另有读数法（§2.11、§3） |
| 2 | **[M1] 真机验收**：Android 低延迟 / 出声延迟 / 30 min 无断流 | §3 | **可执行**（30 min 有现成脚本）；「出声延迟」复用 §2 的量法 |
| 3 | **[M3] 真机验收**：双机同期录音 ±10 ms / 漂移抖动 / ≥ 8 台压测 | §4 | ①②**可执行**（量具已就绪且自检过）；③ **需要 ≥ 8 台真机**，本手册给降级路径（2–3 台） |
| 4 | **[M4] 真机验收**：PC ↔ 手机双向回环 + Android 内录/麦克风清单 | §5 | ⚠️ **双向的一半当前不可执行**：Android 侧**没有连接/推流入口**（见 §6.4，本轮核实）；内录/麦克风清单**可执行**（纯真机操作 + 读数） |

**当前四条共同的硬阻塞**（详见 §6）：手机与 PC **不同子网**、`adb forward` **仅 TCP**、MIUI `adb install` **-99**、**Android 无发送入口**（本手册新增的第 4 条）。

---

## 1. 共用准备（所有真机项都要）

### 1.1 设备与网络（先把这三件事解决，否则后面全是白跑）

| 项 | 要求 | 现状 / 处置 |
|---|---|---|
| Android 设备 | API 29+ 可听声（内录需 API 29+；低延迟播放需 API 26+） | 历史用过 MI 8 Lite（API 29）与 PHK110（API 36） |
| **同一子网** | 手机与 PC 必须在**同一网段**（前三段相同） | 🔴 现成阻塞：手机 `192.168.31.231/24` vs PC `192.168.3.200/24` → **不可达**（跨网段既发现不到、直连也不通，见 `docs/manual/troubleshooting.zh-CN.md`）。处置：让手机接 PC 所在的 AP/路由器 |
| adb | USB 调试可用 | `adb devices` 应列出设备；⚠️ **`adb forward` 只支持 TCP**，而本项目的音频与控制走 **UDP 58290** → **不能用 `adb forward` 把 UDP 端到端搬过来**，只能靠同子网 / USB 反向网络共享 |
| MIUI 安装 release 包 | 开发者选项里打开「USB 安装」 | 🔴 `adb install` release 包报 **`Failure [-99]`**（debug 包可装），见 `docs/12` §8.5 |
| 权限 | 麦克风（`RECORD_AUDIO` 运行时）、MediaProjection（每次会话重新授权）、通知 | 见 §5.2 的清单 |

### 1.2 构建与安装（命令照抄；⚠️ 未经实机验证 = 本轮没跑）

```powershell
# 1) 交叉编译 Rust 内核 → jniLibs
pwsh android/scripts/build-rust.ps1

# 2) 组装 APK（debug 包最快；release 需 keystore.properties，见 docs/12 §8.5）
pwsh tools/gradlew.ps1 -JavaHome '<JDK 17 根目录>' assembleDebug

# 3) 装 + 启动
adb install -r android/app/build/outputs/apk/debug/app-debug.apk
adb shell am start -n com.gotkicry.audiolink.debug/com.gotkicry.audiolink.MainActivity
# 手机界面上点「启动并开始接收」；等服务起来后确认监听 58290
adb shell ss -lun 2>/dev/null | grep 58290   # ⚠️ 部分机型受限；更稳的是直接看 App 界面上的监听状态
```

### 1.3 三条常用读数（真机项的证据来源）

```powershell
adb shell dumpsys media.audio_flinger > target\evidence\m1-device\flinger.txt   # 采样率/格式/FAST/FrmCnt/Latency
adb shell uiautomator dump /sdcard/ui.xml; adb pull /sdcard/ui.xml target\evidence\  # UI 原值（⚠️ 界面每 500 ms 刷新，dump 可能拿不到 idle → 改用 screencap）
cargo run -q -p audiolink-tools --bin latency-probe -- probe <手机IP>:58290 --count 50 --interval-ms 100  # 链路 RTT（10 Hz；1 Hz 会被省电策略拖到 100 ms 量级，见 docs/12 §1.1）
```

---

## 2. [M1] 验收：声学端到端 P50 / P95（★ 全项目最缺的一块）

### 2.1 量什么（口径先钉死，否则数字没法比）

**验收线**：`docs/05` §M1 验收表 —— `e2e_latency_us` **P50 ≤ 110 ms、P95 ≤ 150 ms**（"声学端到端"）。

**这条线指的是**：PC 侧**音频发出** → 经 AudioLink 全链路（采集 → 编码 → QUIC → 解码 → 手机待播队列）→ **手机扬声器出声** → 被测量设备收到。
**不是**：回环测试里「帧封口 → sink 写出」的 40 ms（`docs/47` §4 明写回环不能替代验收：没有 Wi-Fi 抖动、没有扬声器缓冲、没有 AudioTrack 路径）。

**已知的账本（`docs/12` §3，修复前口径）**：采集半周期 10 + 组帧 20 + 网络单向 5.86 + 对端播放环水位 80 + 对端 AudioTrack 80.1 ≈ **195 ms**；
修复后按 `docs/16` 模型下限 ≈ 73 ms **再加设备输出 80 ms ≈ 153 ms** → **仍超 110 ms**。
⇒ 本测量的目的不只是「确认不达标」，而是**给出真实 P50/P95，并按 §2.7 的误差表把它拆开**（哪一段吃掉了预算）。

### 2.2 激励信号（可直接生成，无需新代码）

用**宽带 click 串**，不要用单一频率长正弦 —— 后者的互相关有周期歧义（会在 lag 差一个周期时同样高，把亚采样对齐骗到隔壁周期，实测偏差散开约 1 ms，见 `docs/27` §2.2）。

**现成生成器**（`sync-measure --self-test --write`）⚠️ 未经实机验证（本轮没跑；命令与产物形状来自源码，`src/bin/sync_measure.rs` 的 `self_test`）：

```powershell
cargo run -q -p audiolink-tools --bin sync-measure -- --self-test --write target/evidence/sync
# 产物：self-test-a.wav / self-test-b.wav
#   48000 Hz · 16-bit PCM · 单声道 · 6 个宽带噪声突发（20 ms 窗、0.4 ms 指数衰减、幅度 0.8）
#   间隔 500 ms · 总长 6×500+1000 = 4000 ms
```

- 用它当**测试轨**（`self-test-a.wav` 即可）：6 个 click、间隔 500 ms —— 正好满足「≥ 6 个、间隔 500 ms」的建议；
- **它同时是量具的自检**：若这一跑不通过（退出码非 0），说明量具本身有问题，**先修量具再测真机**；
- 若现场需要更长的序列/更大的间隔，用 ffmpeg 或 python 生成同形状的 click 串（**形状要求**：宽带瞬态 + 快速衰减，间隔 >> 预期延迟）。

### 2.3 采集拓扑（推荐方案：**同源双路出声 + 单支麦克风**）

**核心思想**：让「参考路径」与「被测路径」在**同一支麦克风的同一次录音**里各留下 6 个 click，且**两条路径共享 PC 侧的播放链**——这样相减时，麦克风延迟、录音启动偏移、PC 播放缓冲、PC DAC **全部被消掉**，剩下的差值就是「手机链路 + 声程差」。

**拓扑（推荐，只需 1 台 PC + 1 支麦克风 + 1 台手机）**

```text
        ┌──────────── PC（播放器放 self-test-a.wav，喇叭出声）────────────┐
        │  喇叭 ──────────(声程 P)──────────►  ┌──────────┐
        │                                      │  麦克风  │──► 一次录音 rec.wav
        │  WASAPI loopback 采集同一路输出 ─┐    └────▲─────┘
        └────────────────────────────────┼─────────┼──────────────────────┘
                                         │         │(声程 M)
                                   AudioLink       │
                                   全链路 ▼        │
                                     手机 ──喇叭───┘
```

- **PC 侧**：用一条命令**同时**做到「从输出端点采集（= 送到手机的内容）」与「从喇叭播出（= 参考声音）」：
  ```powershell
  # ⚠️ 未经实机验证。要点：--capture wasapi 采集的就是「PC 正在播放的那一路」，
  # 所以「送给手机的音频」与「PC 喇叭放出的声音」是同一路、同一启动时刻（这是本方案能对消误差的前提）。
  cargo run -q -p audiolink-tools --bin device-link -- run ^
      --peer <手机IP> --seconds 60 --capture wasapi --device id:<PC 输出端点 id> --pin-file target/evidence/pin.txt
  ```
  端点怎么查：`cargo run -q -p audiolink-tools --bin self-loop -- list`（`docs/06` §4.2 点名的复测入口），或桌面端「采集端点选择器」（`docs/13`）。
  `--device` 的选择器形式：`default | id:<子串> | name:<友好名>`（`self_loop.rs` 的 usage）。
- **手机侧**：接收并播放（AudioLink 的既有路径）。
- **麦克风侧**：录**一整段**（覆盖整个 60 s）。录音工具项目里**没有**（`audiolink-tools` 的 bin 列表：alp2-dump / latency-probe / self-loop / link-loop / device-link / soak-runner / sync-measure / netem-sim / license-audit —— **没有任何"录 WAV"的工具**），
  用 ffmpeg（本机已装 `ffmpeg 8.0.1`）：
  ```powershell
  ffmpeg -list_devices true -f dshow -i dummy        # 先查麦克风设备名（名字因机器而异）
  ffmpeg -f dshow -i audio="<麦克风设备名>" -ar 48000 -ac 1 -t 75 target/evidence/m1-acc/rec.wav
  ```

**录音里长什么样**：每个 500 ms 周期里会出现**两个 click**（先 PC 喇叭的、后手机喇叭的，相隔 ≈ 端到端延迟 100–200 ms）。
⇒ **可直接量**：在 Audacity（或 python 读 WAV）里读这两个峰的**时间差**，就是「手机链路相对 PC 直连的净增量」。

**精度够不够**：验收阈值是 110 / 150 ms，而**人工读峰的精度是 1–2 ms**（48 kHz 下一个样本 20.8 µs，读数误差主要来自人眼与包络形状）→ **对判读毫无影响**。
⇒ **M1 这一条不需要亚采样精度**；`sync-measure` 在这里是**可选精化**（想要可复核的 JSON 报告时才用，见 §2.6）。

### 2.4 时间轴对齐（为什么必须"同一次录音"）

- 两段**分别采集**的录音**不能**直接相减：它们的「录音起点」不同，且这个起点差**未知**（麦克风/声卡启动延迟、人对表误差）→ 差值里会混进这个未知量。
- `sync-measure` 的算法**明确假设两段录音同期**：它对两段各自检测脉冲，然后**按序号配对**（第 i 个对第 i 个）并求位置差（`syncmeasure::measure`：`onsets_a[i]` ↔ `onsets_b[i]`）→ 若两段起点不同，这个差没有意义。
- 本方案之所以成立，是因为**参考与被测在同一次录音里**（同一支麦克风、同一条录音流）→ 时基天然一致 ✔

### 2.5 从录音里定位「发出」与「听到」（两种做法）

**做法 ①（推荐，够用）：人工/脚本读两个峰的位置差**
- 「发出」在本方案里 = **PC 喇叭放出的那个峰**（它同时代表"音频已从 PC 输出"）；
- 「听到」= **手机喇叭放出的那个峰**（经 AudioLink 全链路 + 手机输出 + 声程 M）；
- 差 = 手机链路 − PC 播放链 + (声程 M − 声程 P)/343。
- ⚠️ **本方案量到的不是「PC 采集封口 → 手机出声」的绝对值**，而是「手机链路相对 PC 直连播放的净增量」；
  两者相差 PC 播放链（共享模式缓冲下限 **22 ms** + DAC ~1 ms，见 `docs/05` §M1 交付物 1 与 `docs/06` §4.2）。
  **报告时必须写清用的是哪个口径**，两个数字不要混读（`docs/54` §2.C 已经因为「两个都叫 e2e 下限」吃过一次口径混淆）。

**做法 ②（可选精化）：切成两段可配对的 WAV，交给 `sync-measure`**
- 周期 500 ms、两个峰都在周期的前 ~250 ms 内 ⇒ 可以按固定窗口切：第 i 个周期的前 250 ms 取「PC 组」、后 250 ms 取「手机组」，各拼成一段（⚠️ **前提**：端到端延迟 < 250 ms；若现场超了就把激励间隔加大到 1000 ms 再切）。
- 切窗/拼接用 ffmpeg 的 `atrim`+`concat`（命令较长，⚠️ **未经实机验证**）：
  ```powershell
  # 示意：每周期 500 ms，取 0–250 ms 当参考、250–500 ms 当被测（i = 0..5）
  ffmpeg -i rec.wav -filter_complex "<12 段 atrim/asetpts 拼成 ref.wav 与 mic.wav 两个输出>" -map "[ref]" ref.wav -map "[mic]" mic.wav
  ```
  **这一步必须现场把滤镜链写对并验证**（拼接后两段应各含 6 个峰、间隔 500 ms、起点对齐）—— 写进 §2.10 的「必须现场定」。
- 然后：`cargo run -q -p audiolink-tools --bin sync-measure -- ref.wav mic.wav --channel 0 --max-bias-ms 10 --json target/evidence/m1-acc/sync.json`
  → 报告里的 `offset_ms`（B 相对 A）+ 250 ms 的窗口偏移 = 端到端净增量。

### 2.6 `tools/sync-measure` 到底能复用多少（第 113 轮建议的那条）

| 能直接用 ✔ | 不能指望 ✗ |
|---|---|
| **脉冲粗检测**（快攻击慢释放包络 + 阈值穿越插值）+ **亚采样精化**（NCC + 抛物线插值）：自检单对误差 **0.010 ms** | **不做绝对声学延迟**：它只给「两段录音的相对偏差」，`docs/27` §5 自己写明「本工具量的是**录音对齐**；要量『出声』偏差需要麦克风与更严格的反射处理」 |
| **按序号配对 + 逐对偏差**、均值/P50/P95/极值、**漂移 ppm 线性回归** | **不做时间轴对齐**：它假设两段同期（起点相同） |
| **拒绝撒谎**：静音 / 纯直流 / 检测不到脉冲 / 脉冲不足 / NCC < 0.5 → 退出码 **3** | **不做声道分离 / 不做切窗**：`--channel` 对两段录音用**同一个**声道索引（`parse_wav(bytes, channel)`），所以「双声道里一个当参考」必须先切成两个文件 |
| **`ambiguous` 标记**（存在几乎等高的竞争峰 → 标记出来，见 `docs/27` §2.2） | **不做 M1 的采集**：录音这一步项目内无工具（用 ffmpeg） |
| 退出码 0/1/2/3、JSON 报告、`--self-test`（自带真值） | —— |

**结论**：**能复用，但只复用它最擅长的「脉冲定位」**；「采集、时间轴、切窗、口径换算」都要按 §2.3–§2.5 自己做。
（M3 的 ±10 ms 才是它的主场：那里需要亚毫秒，且两段录音天然同期，见 §4.1。）

### 2.7 已知误差来源与量级（写报告时必须逐项交代）

| # | 误差源 | 量级 | 在本方案里的处理 |
|---|---|---|---|
| 1 | **PC 播放链**（共享模式缓冲下限 **22 ms** + DAC ~1 ms） | ~23 ms | **被对消**（参考与被测共享它）；但因此**量到的不是"PC 采集封口→手机出声"的绝对值**，报告要写清口径 |
| 2 | 麦克风采集延迟 / 录音启动偏移 | 未知（数 ms–数十 ms） | **被对消**（同一次录音、同一设备） |
| 3 | 两个声源到麦克风的**声程差** | 10 cm ≈ **0.29 ms**；1 m ≈ 2.9 ms | 量一下距离写进报告；近距离可忽略 |
| 4 | 手机**输出侧**（本次要被量到的对象） | AudioTrack 实际缓冲 **3844 帧 = 80.08 ms**（请求 960 帧被框架顶到）；flinger 报 track latency **101 ms** | 属被测对象，不扣；报告里与 `docs/12` §2.1b 的数字对照 |
| 5 | 手机**待播队列水位** | 实测 40–200 ms 随运行增长（`docs/12` §3 与 §5.1：30 min 时 P50 200 ms、顶到 320 ms 上限） | 属被测对象；测前记录当轮水位（遥测面板） |
| 6 | 环境**混响/反射** | 首达峰之外的反射峰可能被 NCC 当成竞争峰 → `ambiguous` | 关窗、近场、单次 click 用宽带瞬态；看到 `ambiguous` 就换环境或加大 `--min-gap-ms` |
| 7 | 网络抖动 | 推流中 §6 RTT P50 11.7 ms / P95 81.1 ms（跨三层）；同子网 P50 6.65 ms（`docs/12` §8.1） | 报告里记录当轮路径形态（同子网/跨三层） |
| 8 | 时钟漂移（两台设备不同源） | ±50 ppm 量级；30 min ≈ 90 ms（若真发生） | 见 `docs/12` §4 #11（机制未查清）；测量用 20–60 s 窗口，漂移影响小 |

### 2.8 判读（阈值写死）+ 失败时最可能的三个原因

| 结果 | 判定 |
|---|---|
| P50 ≤ **110 ms** 且 P95 ≤ **150 ms** | ✅ M1 延迟验收**达标**（样本 ≥ 6 对；报告写明口径与误差项） |
| P50 > 110 ms 或 P95 > 150 ms | ❌ **不达标**，按下面三因排查 |

**失败时最可能的三个原因（按本轮账本的概率排序）**：
1. **手机侧播放链吃满**：AudioTrack 实际缓冲 80.08 ms（请求 960 帧被顶到 3844 帧）+ flinger track latency 101 ms —— 这是 `docs/12` §2.1b 的实测，且 `§4 #9` 指出「内核 → Kotlin PCM 推送 ≈ 2× 实时」会让延迟搬进内核队列；**先看这两处**，别急着怀疑网络。
2. **手机待播队列水位**：30 min 档实测 P50 200 ms / 顶到 320 ms 上限（`docs/12` §5.1）→ 水位不降下来，任何链路优化都填不满 110 ms。
3. **网络路径形态**：跨三层（PC → 192.168.3.1 → … → 手机）时 §6 RTT P50 11.7 ms / P95 81.1 ms；同子网 P50 6.65 ms（`docs/12` §8.1）→ **先确认同子网**（§1.1），再谈数字。

### 2.9 证据留存（`target/evidence` 不入库，所以这样做）

```text
target/evidence/m1-acoustic/<日期>/
  rec.wav                # 原始录音（可能几 MB，不入库）
  sync.json              # 若走了 sync-measure 路线
  notes.md               # 设备/系统/包名/git hash/距离/音量/房间/时段的原始记录
```
- 落文档的最小集：**P50/P95、口径（含不含 PC 播放链）、样本对数、设备与版本、三类误差项的量级、rec.wav 与 sync.json 的 SHA-256**；
- 写回位置：`docs/12-m1-device-acceptance.md`（新开一节）或 `docs/16`（PHK110 复测口径）。

### 2.10 哪些必须现场定（**不编造**：这些本机定不了）

1. **麦克风与两个声源的位置、音量、房间**（决定信噪比与反射）；录音设备名（`ffmpeg -list_devices` 现查）。
2. **是否具备「同时录 line-in + 麦克风」的硬件**（USB 声卡）。若有，可用**更干净的第二拓扑**：CH1 = 线路输入接 PC 耳机输出（参考），CH2 = 麦克风（手机声音），同一文件的**两个声道**天然同期 → 分离声道后喂 `sync-measure`（`--channel` 对两段用同一索引，所以必须切成两个文件）。本方案**不依赖**该硬件，但能显著简化切窗。
3. **切窗参数**（周期、窗口宽度）与 ffmpeg 滤镜链能否正确工作（§2.5 做法②）。现场用 `ffprobe`/Audacity 核对「两段各 6 个峰、间隔 500 ms、起点对齐」。
4. **是否存在可靠的"发送信号"参考**：本轮推荐用「PC 喇叭出声」当参考（自带对消），代价是口径偏移 PC 播放链（22 ms 量级）；
   若现场要求**严格等于**「PC 采集封口 → 手机出声」的口径，需要额外用 WASAPI loopback 数字域参考 + 精密的通道间对齐（本机无法验证，**必须先做一次"参考通道 ↔ 麦克风通道"的标定实验**）。
5. **当轮软件版本**（git hash）与手机包名/profile（debug 或 release —— 两者的 R8/优化不同，延迟可能不同，不要在测量中途换包）。

### 2.11 同一条验收里的另外两项（读数即可，不需要声学）

| 项 | 判据 | 方法（本轮已核实：断言 + 单测 + 真机读数） |
|---|---|---|
| **零重采样** | 全链路 48 kHz，不存在 SRC | 引擎侧 `format_guard::require_unified_format` / `require_unified_link` 是**断言**（不达标直接 `Err` 打断启动），7 项单测；真机侧看 flinger：`PCM_FLOAT` / 48000 / 2ch / `PRIMARY|FAST`（`docs/12` §0） |
| **低延迟模式生效** | `getPerformanceMode() = LOW_LATENCY` | `adb shell uiautomator dump` 读 UI 原值 + flinger 片段；历史两代设备都有读数，**"今天是否仍成立"必须真机复测** |


---

## 3. [M1] 真机验收：Android 低延迟 / 出声延迟 / 30 min 无断流

看板原文口径（**准确**，`docs/54` 已核）：`仍缺修复后 30 min 长跑和声学出声延迟 P50/P95`。

### 3.1 低延迟模式生效（读数，不靠听感）

```powershell
adb shell uiautomator dump /sdcard/ui.xml; adb pull /sdcard/ui.xml target\evidence\m1-device\
adb shell dumpsys media.audio_flinger > target\evidence\m1-device\flinger.txt
```
判据：`getPerformanceMode() = LOW_LATENCY`、主输出 `PRIMARY|FAST`、`FrmCnt 3844`、`Underruns 0`、FastMixer 活跃（`MIX_WRITE` 递增）。

### 3.2 出声延迟 P50/P95

= **§2 的声学量法**（同一条线，同一套判读）。真机报告里把「低延迟模式读数」与「声学 P50/P95」**分开写**（一个是配置事实、一个是物理结果）。

### 3.3 修复后 30 min 无断流（有现成脚本）

```powershell
# ⚠️ 未经实机验证（本轮真机不在场）。脚本存在但 target/evidence 是 gitignored：
#     若已丢失，用下面的替代命令直接跑 1800 s。
pwsh target/evidence/acceptance/soak-30min.ps1 -Seconds 1800 -Dir target/device-link-3 -Tag soak30

# 替代（脚本不在时）：
cargo run -q -p audiolink-tools --bin device-link -- run --peer <手机IP> --seconds 1800 ^
    --capture synth --pin-file target/evidence/pin.txt --json target/evidence/m1-device/soak30.json
```

**判据（全部要，缺一不算过）**：
1. 全程 `streaming`，**无断连、无重连**（会话状态与会话表数）；
2. 丢包**末值 0.00%**（个别 1 s 窗口允许瞬时尖峰，记录最大值）；
3. **`late_drops` 与 `underruns` 取增量而不是累计值**（`docs/12` §7 踩过「把首值当读数」的坑）；
4. **队列水位不持续上涨**（此前的失败形态就是从 160 ms 一路顶到 320 ms 容量上限并振荡）；
5. PLC = 0、码率稳定在目标附近。

> ⚠️ **口径警告**：`docs/12` §5.1 那次 30 min 是**队列修复之前**的版本（水位顶到 320 ms、迟到丢弃 53、欠载 7→69），
> 而它当时**没有标注「修复前」**（`docs/54` §2.B 已点名）。本次跑出来的是**修复后**的数字，报告里必须写清是哪一版。

**失败时最可能的三个原因**：
1. **接收侧水位上涨**（`docs/12` §4 #9：内核 → Kotlin PCM 推送 ≈ 2× 实时；§4 #11：速率失配，机制仍未查清）；
2. **瞬时丢包**（30 min 里最大 1 s 窗口曾到 6.25%）→ 看 `docs/22` 的判定口径；
3. **系统侧**（MIUI 省电/后台限制杀掉采集或服务；前台服务类型位是否被系统接受，见 §5.3 #4）。

### 3.4 证据留存

`target/evidence/m1-device/`：`soak30.log`（逐秒）、`soak30.json`（账本）、`flinger.txt`、`ui.xml`、`notes.md`。
落文档：会话存活结论 + 水位/迟到/欠载的**增量** + 设备与版本 + 文件 SHA-256。

---

## 4. [M3] 真机验收：三条

看板原文（第 107 轮按「实现 + 自动化证据」判 Done 后另立的真机条目）：
① **双机同期录音验证组内 ±10 ms**（口径提示：回环量的是播放回调时刻、不是扬声器波形，**不可能靠回环替代**）；
② **晶振漂移下的 offset 抖动**（两台设备时钟不同源）；③ **≥ 8 台真机压测**。

### 4.1 双机同期录音：组内 ±10 ms（**本手册里可执行度最高的一条**）

量具已就绪且自检过（`sync-measure --self-test`：单对最大误差 0.010 ms、漂移 1.0 ppm），步骤照 `docs/27` §4：

```powershell
# 1) 两台接收端加入同一个同步组（桌面「同步组」面板勾选成组），播放同一条含宽带 click 的测试轨
#    （测试轨可用 §2.2 的 self-test-a.wav：6 个 click、间隔 500 ms）
# 2) 各自录下自己的输出（⚠️ 需要两套录音设备/两条线路录制；现场定），采样率统一 48 kHz
# 3) 跑量具
cargo run -q -p audiolink-tools --bin sync-measure -- phone-a.wav phone-b.wav --json target/evidence/sync/m3.json
```
**判据（写死）**：`verdict = within` ⇒ `|均值|` 与 `P95(|·|)` 均 ≤ **10 ms**，且漂移 ≤ **200 ppm**；退出码 0。

**偏差偏大时怎么读**（`docs/27` §4）：
- **逐对偏差有趋势** ⇒ 晶振**速率差**（后续靠 `drift_ppm` 做速率微调）；
- **没有趋势** ⇒ **固定延迟差**（后续靠每设备手动延迟补偿，`docs/05` §M3 风险项里那个 ±50 ms 滑块 —— **目前未实现**）。

**失败时最可能的三个原因**：
1. **起播错一帧**（20 ms）：这条曾被定位到**三层**根因并修掉（`docs/49` §7.5–§7.7；第三层是「target 网格对齐 + 相位」），**真机是否仍复现要实测**；
2. **两台设备的输出延迟不同**（扬声器/DAC/缓冲差异）→ 表现为固定偏差，**当前没有补偿入口**；
3. **录音不同期 / 采样率不一致 / 反射峰**：`sync-measure` 会明确报错（退出码 3）或标 `ambiguous` —— **不要把它当成"通过"**。

### 4.2 晶振漂移下的 offset 抖动 ≤ 2 ms（遥测读数）

- 用桌面遥测面板 / CSV 导出，连续采 **≥ 10 min** 的 `clock_offset_us` 与 `drift_ppm`；
- 判据：**稳定后 offset 抖动 ≤ 2 ms**（`docs/05` §M3 验收表第 4 条）；
- 参考基线：回环 12 s 实测抖动 **3 µs**（`docs/33` §8）—— 那不含晶振漂移，真机才是这条线的主场；
- 同时记录 `drift_reliable` 与 `drift_ppm`（30 min soak 留过 `drift_ppm = 2` 的现成基线）。

**失败时最可能的三个原因**：① 探针节奏太慢（1 Hz 会被无线省电拖到 100 ms 量级，见 `docs/12` §1.1）→ 用推流中的 10 Hz 口径；② 会话中途重连/重排；③ 设备侧省电降频（`drift_reliable=false`）。

### 4.3 ≥ 8 台真机压测（**设备门槛最高**）

- 本机 8 台已由 `multi_session::one_capture_feeds_eight_receivers` 覆盖（8 台各 470 400 非静音样本、排播各 1）；
- **真机**要 ≥ 8 台 —— 拿不到就用**降级路径**并如实标注：
  - 3–4 台真机 + 1 台回环，判据：**每台都真的出声（非静音样本数）** + **每台都进 epoch 排播** + **各台播放量同量级（彼此差 ≤ 一成）**；
  - 台数不足时**不要**写「8 台压测通过」，写「N 台通过，8 台未测」。
- 失败时最可能的三个原因：① AP 带宽/客户端隔离；② 发送端单机 CPU（多路编码）；③ 混音器 8 路源号上限（FR-12 的 `STREAM_LIMIT`）。

---

## 5. [M4] 真机验收：双向回环 + Android 内录/麦克风清单

### 5.1 PC → 手机（可执行）

用 `device-link`（`docs/12` §6）或桌面 App：「连接 → 开始推流 → 手机出声」。

### 5.2 手机 → PC（⚠️ **当前不可执行 —— 本轮核实的新发现**）

**核实方式**：在 `android/app/src/main/kotlin/` 全目录搜 `connect` / `start_send` / `startSend` → **零命中**；
Android 侧只有**接收播放**（`PcmFeed` → `PcmRingBuffer` → `LowLatencyPlayer` → `AudioTrack`）与**采集供数**（`CaptureController` → `PcmPull` 交给内核）。
⇒ **手机作为发起端连接 PC、并触发推流，当前没有入口**（既没有 UI，也没有调用点）。
另有面板已记的 `[M4] 能力位：EngineStartConfig.capabilities…`（**Todo**）：Android 无法把「支持内录/麦克风」告诉对端。

**结论**：「PC ↔ 手机双向」这一条**不是"缺证据"，而是"缺入口"** —— 先补实现（连接入口 + `start_send` 触发 + `capabilities` 字段），再谈验收。
在此之前只能测「PC → 手机」单向，**不要**把单向结果写成双向达标。

### 5.3 Android 内录 / 麦克风真机清单（第 106 轮由实现方列出；**不得当成已验证**）

逐条给「怎么判」：

| # | 检查项 | 判读方式 | 期望 |
|---|---|---|---|
| 1 | 内录是否真抓到**系统音频** | 手机上播放已知内容（含 `allowAudioPlaybackCapture=false` 的应用与 DRM 内容各一），PC 侧录音/看遥测是否收到非静音 | 可捕获的应用有声音；被禁的**明确说明**（`docs/05` §M4 风险项） |
| 2 | 麦克风实际**采样与延迟** | 采集状态 + 日志/读数（`CaptureController` 的状态机有状态输出） | 48 kHz / 2ch；延迟记录在案 |
| 3 | MediaProjection **授权弹窗** + Android 14+ **每会话重授权** | 每次点「系统内录」都应弹授权；拒绝后走「等授权」而非永久失败 | 弹窗出现；重授权可重复（`SystemLoopbackCaptureSource` 注释写明系统行为） |
| 4 | `microphone` / `mediaProjection` **前台服务类型位在 MIUI 上是否被接受** | 采集期间看通知是否显示、服务是否被杀；`dumpsys activity services` | 服务存活、通知可见 |
| 5 | `AudioRecord` **被占用**时的状态检查路径 | 先用别的 App 占住麦克风，再启动采集 | 报「设备被占用」而不是静默无声（`AudioRecordSupport.startOrFail` 的状态检查） |
| 6 | UniFFI **装箱（1920 样本/帧）的 GC 抖动** | 长跑中看是否有周期性欠载（手机侧计数） | 无明显周期性掉帧 |

### 5.4 失败时最可能的三个原因（M4 整条）

1. **内录被目标应用限制**（`allowAudioPlaybackCapture` / DRM）→ 属预期，UI 必须说明；
2. **前台服务类型被系统拒绝**（MIUI 更可能）→ 采集起不来或很快被杀；
3. **授权被收回**（`CaptureError::PermissionRevoked`）→ 内录**每次会话都要重新授权**（系统行为）。

---

## 6. 已知阻塞与解除路径

| # | 阻塞 | 现象 | 解除路径 | 出处 |
|---|---|---|---|---|
| 1 | **手机与 PC 不同子网** | 手机 `192.168.31.231/24` vs PC `192.168.3.200/24` → 发现不到、直连不通 | 让手机接 PC 所在的路由器/AP；确认前三段一致后先 `ping` | 看板 `[M3] 真机验收` 条目；`docs/manual/troubleshooting.zh-CN.md` §「两台设备不在同一网段」 |
| 2 | **`adb forward` 仅支持 TCP** | 音频与控制走 **UDP 58290** → 不能靠它搬链路 | 用同子网（首选）；或 USB 反向网络共享（USB tethering）让 PC/手机处于同一网段 | 看板同条目 |
| 3 | **MIUI `adb install` release 包 `Failure [-99]`** | debug 包可装、release 被拒 | 开发者选项打开「USB 安装」（环境项） | `docs/12` §8.5 |
| 4 | **Android 无「连接/推流」入口**（本手册新增） | 手机不能作为发送端（§5.2） | 需补实现：连接入口 + `start_send` 触发 + `EngineStartConfig.capabilities` | 本轮核实（Kotlin 全目录 `connect`/`start_send` 零命中）+ 看板 `[M4] 能力位…`（Todo） |

---

## 7. 未经实机验证清单（集中版，按章节点名）

| 章节 | 未验证的内容 | 为什么 |
|---|---|---|
| §1.2 | 全部构建/安装/启动命令 | 本轮真机不在场（`adb devices` 空） |
| §1.3 | `uiautomator dump` 在目标机型的可用性 | 历史记录过「界面每 500 ms 刷新 → 拿不到 idle」 |
| §2.2 | 用 `self-test-a.wav` 当测试轨 | 命令与产物形状取自源码，未在真机上播过 |
| §2.3 | 拓扑与 ffmpeg 录音命令、设备名 | 依机器而异；`-list_devices` 现场查 |
| §2.5 | 做法② 的 ffmpeg 切窗/拼接滤镜链 | 需要现场写对并用 `ffprobe`/Audacity 核对 |
| §2.8 | 三个失败原因的概率排序 | 依据是 `docs/12`/§5.1 的历史账本，不是本轮新测 |
| §3.3 | `soak-30min.ps1` 的可用性与修复后 30 min 结果 | 脚本是 gitignored 的本地产物；修复后从未跑过 |
| §4.1/§4.2 | M3 双机录音与漂移抖动的真机读数 | 工具自检过，但**从未在真机上量过** |
| §4.3 | ≥ 8 台真机 | 没有 8 台设备 |
| §5.2 | 「手机 → PC」 | **缺实现入口**（不是缺证据） |
| §5.3 | 内录/麦克风 6 项 | 第 106 轮由实现方列出，**从未真机跑过** |

---

## 8. 速查

**阈值**

| 项 | 阈值 |
|---|---|
| M1 声学端到端 | P50 ≤ **110 ms** / P95 ≤ **150 ms** |
| M1 出声 | 连接成功 ≤ **300 ms** 内出声 |
| M3 组内同步 | \|均值\| 与 P95 ≤ **10 ms**、漂移 ≤ **200 ppm** |
| M3 时钟抖动 | 稳定后 offset 抖动 ≤ **2 ms** |
| 回环护栏（**不是验收**） | P50 ≤ 80 ms / P95 ≤ 150 ms（`docs/47`） |

**常用命令**

```powershell
# 链路 RTT（10 Hz，别用 1 Hz）
cargo run -q -p audiolink-tools --bin latency-probe -- probe <手机IP>:58290 --count 50 --interval-ms 100
# 真机推流（synth / 真实 WASAPI）
cargo run -q -p audiolink-tools --bin device-link -- run --peer <手机IP> --seconds 60 --capture synth
# 本机自环（回环基线，非验收）
cargo run -q -p audiolink-tools --bin self-loop -- run --seconds 20
# 量具自检（先证明量具准，再量真机）
cargo run -q -p audiolink-tools --bin sync-measure -- --self-test --write target/evidence/sync
# 双机录音对齐（M3）
cargo run -q -p audiolink-tools --bin sync-measure -- a.wav b.wav --json target/evidence/sync/m3.json
# 录音（外部依赖：ffmpeg；项目内没有录 WAV 的工具）
ffmpeg -f dshow -i audio="<设备名>" -ar 48000 -ac 1 -t 75 out.wav
```

**做完了记得**：把结论（含口径、样本数、设备版本、文件 SHA-256）写回 `docs/12` / `docs/16` / `docs/33` 对应章节 —— `target/evidence/` 会被清掉，**只有文档里的结论能留下来**。
