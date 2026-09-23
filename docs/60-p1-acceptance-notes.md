# P1 验收记录 —— 声学延迟量具这一半（tools/acoustic-latency.ps1）

> ⚠️ **历史记录说明（2026-09-22 补）**：本文记录的是 **PIN 配对 / 信任库时代**的交付事实与证据。
> 该认证机制已在本轮整体移除（现在连上即用：没有配对码、没有白名单），本文中的配对步骤、PIN 验收数字与
> 信任判定均**不再对应当前实现**，只作为当时的交付记账保留 —— 现状见 `docs/72-remove-pairing.md`。

> 作者：**acoustic-tooling**（共享任务 **task-2**）｜ 日期 2026-09-18 ｜ 工作区：C:\_Project\AudioLink
> 量法出处：**docs/55-device-acceptance-runbook.md §2**（写死，不改；本脚本只是它的可执行版本）
> **本文不含任何未实测的延迟数字**：所有数字都注明来源（本机自检 / 本机设备探测）。

---

## 0. 一句话状态

| 这一半 | 状态 | 证据 |
|---|---|---|
| 脚本（生成激励 → 播放+录音 → 定位 → P50/P95 → 判读 → 落盘） | ✅ **完成** | [tools/acoustic-latency.ps1](tools/acoustic-latency.ps1)（1482 行，SHA-256 90e3fa74c81bba13…） |
| 自检（不需要麦克风，证明定位/配对/分位/判读正确） | ✅ **14/14 通过**（19.8 s） | target/evidence/m1-device-p1/acoustic-selftest.log（SHA-256 7187c1a979dba22034dcf52d1f473051cd3c75b7b3b9d9cac937251e86401ae5） |
| 预演 `-DryRun`（步骤 / 命令 / 人工清单 / 误差表） | ✅ 完成 | 同日志 |
| 缺麦克风时**明确失败**（不输出任何伪造数字） | ✅ 实测退出码 **4**（DryRun 与 Run 都测了） | 本机探测 -91.0 / -90.3 dB ⇒ 判「拿不到声学信号」；Run 在探测处即退出，**没有**启动 device-link、**没有**播放、没有产生任何数字 |
| **现场那一半**（插物理麦克风 + 摆位 + 手机链路） | ❌ **未做** | 本机没有物理麦克风输入端点 |

---

## 1. 这段做了什么 / 没做什么（边界先钉死）

**做了**：量具本体 + 自检 + 人工动作清单 + 离线分析入口。脚本一条命令走完「生成激励 → 校验麦克风 → 打印摆位清单 → device-link 推流 → 播放 + 录音（同一次录音、同一时间轴）→ 定位「发出/听到」→ P50/P95 与样本数 → 落盘录音/读数/判读」。

**没做（也不该由脚本做）**：插麦克风、摆位、调音量、确认同子网、手机 App 的接收操作。脚本**不碰 adb、不碰手机**（设备由另一名成员独占）。

**没验证**（必须诚实交代）：Run 模式里「device-link 推流 + ffplay 播放 + ffmpeg dshow 录音」三者并发的**现场时序**，本机没有条件跑（无麦克风、无手机）。脚本里这条路径的每一步都标了 ⚠️。自检验证的只是**分析链路**（定位/配对/分位/判读），这一点在 §3 说清。

---

## 2. 用法（三条命令 + 一条可选）

```powershell
# ① 自检（不需要麦克风；跨机器可复跑，约 20 s）
pwsh tools/acoustic-latency.ps1 -SelfTest

# ② 预演：打印步骤、将要执行的命令、现场清单、误差表；对麦克风只做 3 s 只读电平探测
pwsh tools/acoustic-latency.ps1 -DryRun -MicDevice "<麦克风 dshow 名>" -Peer <手机IP>

# ③ 正式测量（现场：插好麦克风 + 手机在同一子网并已开始接收）
pwsh tools/acoustic-latency.ps1 -Run -MicDevice "<麦克风 dshow 名>" -Peer <手机IP> -ExtraPathCm <声程差 cm>

# ④ 可选：离线分析一段已有录音（同一次录音里含「参考峰 + 被测峰」）
pwsh tools/acoustic-latency.ps1 -AnalyseWav <rec.wav>
```

**查麦克风名**（名字因机器而异，必须完全一致，含中英文括号）：

```powershell
ffmpeg -hide_banner -list_devices true -f dshow -i dummy
```

**先预热 device-link**（本项目 `.cargo/config.toml` 锁了 `build.target = x86_64-pc-windows-msvc` ⇒ 产物在 `target\x86_64-pc-windows-msvc\debug\`；**本机该 exe 已存在**，脚本会直接找到它。只有两个位置都找不到时才回退到 `cargo run`，那种情况下现场首跑要等编译）：

```powershell
cargo build -p audiolink-tools --bin device-link
```

### 参数速查

| 参数 | 默认 | 说明 |
|---|---|---|
| `-MicDevice` | 无（必填） | dshow 输入设备名，逐字一致 |
| `-Peer` | 无（Run 必填） | 手机 IP[:端口]；端口省略 = QUIC 58290 |
| `-PcDevice` | `default` | device-link 的 WASAPI 渲染端点选择器（`default` / `id:<子串>` / `name:<友好名>`）；**必须与系统默认输出端点一致**（ffplay 只往默认端点放） |
| `-ExtraPathCm` | 0 | 两个声源到麦克风的**声程差**（cm）；10 cm ≈ 0.29 ms |
| `-Rounds` | 1 | 重复几轮「播放+录音」，样本合并后统一算分位 |
| `-MinPairs` | 6 | 有效配对下限；不到这个数一律判「检测失败」，不给数字 |
| `-MicSilenceDb` | -80 | 麦克风可用性判据：`max_volume <= -80 dB` ⇒ 该端点拿不到声学信号 |
| `-EvidenceDir` | target/evidence/m1-device-p1 | 证据目录（target/ 不入库） |
| `-NonInteractive` | 关 | 不等待人工回车（无人值守用） |

---

## 3. 自检证据（`-SelfTest`）

**判据**：每个用例的读数与合成真值之差 **≤ 1 ms**；反例必须失败**且不输出任何数字**。

| 用例 | 内容 | 结果 |
|---|---|---|
| T1-stimulus-shape | 激励 = 48 kHz / 16-bit / 单声道、6 个 20 ms 突发、间隔 500 ms、总长 4.000 s、峰值 0.768、间隙严格为 0 | PASS |
| T2A-const-92 | Δ = 92.0 ms 常数 | PASS：读数 P50 **92.0** / P95 92.1，单对最大误差 **0.155 ms**，判定 pass |
| T2B-const-1184 | Δ = 118.4 ms 常数 | PASS：读数 P50 118.4，判定 **fail**（P50 > 110，走 P50 分支） |
| T2C-ramp-95-145 | Δ 从 95 线性升到 145 | PASS：读数 P50 115.1 / P95 145.1，判定 fail |
| T2D-jitter-88-108 | Δ 抖动 + 底噪 -50 dB + 反射增益 0.45 | PASS：读数 P50 95.6，单对最大误差 0.153 ms |
| T2E-weak-mic-relax | 被测峰只有参考峰的 28%（默认阈值下会被吞掉） | PASS：阈值自动降级到 **0.18** 后测出 P50 100.0，**降级写进报告** |
| T2F-p95-branch | Δ = 100,100,100,100,100,160 | PASS：P50 100.0（≤110）但 P95 **160.1** ⇒ 判定 fail（走 P95 分支） |
| T2G-large-300ms | Δ = 300 ms（大延迟边界） | PASS：读数 P50 300.0，判定 fail（未被当成「无信号」） |
| T4-late-recording + T4b | 真值同 T2A，但录音起点整体后移 1.4 s | PASS：**P50 91.978 ms → 91.978 ms（差 0.000 ms）** ⇒ §2.4「同一次录音」纪律在实现上成立 |
| T3A-silence | 全零录音 | PASS（反例）：明确失败「录音是静音（包络峰值为 0）」 |
| T3B-ref-only | 只有 PC 喇叭那 6 个峰 | PASS（反例）：有效配对 0 对 ⇒ 失败 |
| T3C-mic-only | 只有手机那 6 个峰 | PASS（反例）：有效配对 0 对 ⇒ 失败 |
| T3D-mic-too-weak | 被测峰弱到 0.06（低于任何降级阈值） | PASS（反例）：失败，**没有输出 P50/P95** |

**关键行原文**（完整日志：target/evidence/m1-device-p1/acoustic-selftest.log）：

```text
[PASS] T2A-const-92 —— 读数 P50 92.0 / P95 92.1 ms（真值 92.0 / 92.0），单对最大误差 0.155 ms，阈值比 0.5，判定 pass
[PASS] T4b-start-shift-invariance —— 录音起点后移 1.4 s：P50 91.978 ms → 91.978 ms（差 0.000 ms，判据 <= 0.05 ms）
[PASS] T3A-silence —— 按时失败且没有数字：录音是静音（包络峰值为 0）—— 没有可比的东西
自检通过：14/14（量具的定位/配对/分位/判读逻辑可用）
未覆盖的部分（必须如实交代）：真实麦克风采集、真实手机链路、PC 播放链与 device-link 的时序 —— 这些本机没有条件验证。
```

### 性能（本机实测）

| 录音长度 | 分析耗时 |
|---|---|
| 4.8 s（自检合成） | 1.0 s |
| **57.6 s**（12 × 合成录音） | **2.2 s** |

⇒ 现场录 8–60 s，分析都在数秒内；长录音里 144 个峰也能正确配出 6 对，另外 **132 个额外峰被明确列出**（不静默丢弃）。

---

## 4. 现场摆位与操作清单（照做即可；出自 docs/55 §2.3 / §2.10）

1. **插上物理麦克风**（USB 麦或 3.5 mm 麦）。本机 dshow 里只有两个虚拟端点，它们拿不到声学信号。
2. 关窗、关风扇/空调；手机静音其它通知；房间尽量安静（反射峰会被当成竞争峰，§2.7 #6）。
3. 麦克风摆在 **PC 喇叭** 与 **手机扬声器** 之间，尽量等距；量出两者到麦的距离差 → 传给 `-ExtraPathCm`。
4. 调音量让两个 click 在录音里**幅度接近**：PC 音量别开大，手机媒体音量适中。
   （判据：包络阈值 = 全局峰值 × 0.5；手机声音弱于 PC 一半时被测峰会被整个吞掉 —— 脚本会自动降阈值并**在报告里留痕**，但优先现场调好。）
5. 手机与 PC **同一子网**（§1.1）；手机 App 启动接收。
6. **系统默认输出端点必须就是 device-link 采集的端点**（`--device default`）：ffplay 只往默认端点放；两者不一致 = 参考声与被测声不是同一路，前功尽弃。测量期间不要切设备、不要插拔耳机。
7. 手机侧连接：本版本已无配对码 —— 手机 App 启动接收后会直接连上 device-link（旧的 `pin.txt` 投喂步骤随 PIN 配对一起移除）。
8. 记录进 notes.md（脚本自动生成骨架）：手机型号/系统/包名、PC 端点名、两端距离、房间、音量、git hash、**手机待播队列水位**。
9. 测完再动手机；期间不要切后台、不要锁屏省电。

---

## 5. 误差来源与量级（docs/55 §2.7 原表，脚本里也打印）

| # | 误差源 | 量级 | 在本方案里的处理 |
|---|---|---|---|
| 1 | PC 播放链（共享模式缓冲 22 ms + DAC ~1 ms） | ~23 ms | **被对消**（参考与被测共享）→ 因此量到的是「**口径 A**」净增量 |
| 2 | 麦克风采集延迟 / 录音启动偏移 | 未知（数 ms–数十 ms） | **被对消**（同一次录音、同一设备）；T4b 用例实测差 0.000 ms |
| 3 | 两个声源到麦克风的声程差 | 10 cm ≈ 0.29 ms；1 m ≈ 2.9 ms | 现场量距离 → `-ExtraPathCm` 修正 |
| 4 | 手机输出侧（**被测对象，不扣**） | AudioTrack 3844 帧 = 80.08 ms；flinger 101 ms | 报告里与 docs/12 §2.1b 对照 |
| 5 | 手机待播队列水位（**被测对象**） | 实测 40–200 ms，30 min 时 P50 200 ms | 测前记录当轮水位 |
| 6 | 环境混响/反射 | 反射峰可能被当成竞争峰 | 报告里列 `extra_peaks_secs`；关窗、近场重测 |
| 7 | 网络抖动 | 跨三层 RTT P50 11.7 / P95 81.1 ms；同子网 P50 6.65 ms | 报告里记录当轮路径形态 |
| 8 | 时钟漂移（两台设备不同源） | ±50 ppm 量级 | 本次窗口 20–60 s，影响小 |
| — | **本量具自身的检测误差** | 单对 **≤ 0.155 ms**（合成真值比对，见 §3） | 对 110/150 ms 的判读无影响 |

### 口径必须写明（否则两个数字会混读）

- 本量具量的是「**手机链路相对 PC 直连播放的净增量**」= **口径 A**；
- **不是**「PC 采集封口 → 手机出声」的绝对值（**口径 B**），两者相差 PC 播放链（22 ms + DAC ~1 ms）；
- 报告 JSON 里的 `calibration` / `scope` 字段会把这一句带上；docs/54 §2.C 已经因为口径混淆吃过一次亏，不要重演。

---

## 6. 判读与退出码

| 退出码 | 含义 |
|---|---|
| 0 | 达标：P50 ≤ 110 ms 且 P95 ≤ 150 ms，且样本对数 ≥ 6 |
| 1 | 不达标（**测到了**但超阈）→ 按 §2.8 三因排查：① 手机播放链吃满 ② 待播队列水位 ③ 网络路径形态 |
| 2 | 用法错误（缺 `-Peer`、证据目录写不了……） |
| 3 | **检测失败：没有数字**（峰数不足 / 录音静音 / 读不了文件） |
| 4 | **设备不可用**（无麦克风 / 端点静音 / 缺 ffmpeg） |

**铁律**：退出码 3/4 时**绝不输出任何 P50/P95** —— 脚本里所有延迟字段一律 `null`，JSON 里也是 `null`。「测具撒谎比测不出更坏」（syncmeasure.rs 的模块注释原话，本脚本沿用同一纪律）。

---

## 7. 本机事实与未验证边界

**本机事实（实测）**：

- dshow 音频**输入**端点只有两个，都是虚拟设备：`Virtual Mic (Virtual Mic for AudioRelay)`、`麦克风阵列 (网易虚拟音频设备)`；
- 对后者做 3 s 只读电平探测：`mean_volume = -91.0 dB / max_volume = -90.3 dB` ⇒ 数字静音，**判为拿不到声学信号**（判据 `max_volume <= -80.0 dB`）；
- ffmpeg / ffplay 8.0.1 可用；`target\x86_64-pc-windows-msvc\debug\device-link.exe` **存在**（6 221 824 B，2026-09-18 11:29）；`target\debug\` 那个路径确实不存在 —— 脚本按「triple 优先、两者都找」解析（Lead 复核时补的一处可用性修复）。

**未验证**：Run 模式的三进程并发时序（device-link 推流 ↔ ffplay 播放 ↔ ffmpeg dshow 录音）、真机链路的实际读数、`ffplay` 与 `device-link` 端点一致性在真机上的效果。这些只能在有麦克风 + 有手机的现场验证。

---

## 8. 用户侧待执行（最小动作集）

```powershell
# 1) 插上物理麦克风，然后查名字
ffmpeg -hide_banner -list_devices true -f dshow -i dummy

# 2) 预热 device-link（可选但强烈建议）
cargo build -p audiolink-tools --bin device-link

# 3) 预演：应先看到「✅ 该端点能拿到声学信号」
pwsh tools/acoustic-latency.ps1 -DryRun -MicDevice "<麦名>" -Peer <手机IP>

# 4) 摆好麦、调好音量、手机 App 开始接收、系统默认输出 = device-link 采集端点，然后：
pwsh tools/acoustic-latency.ps1 -Run -MicDevice "<麦名>" -Peer <手机IP> -ExtraPathCm <声程差cm>
```

跑完后把 `target/evidence/m1-device-p1/<时间戳>/` 下的 `acoustic-latency.json` / `notes.md` / `rec.wav` 交给 Lead —— **结论要按 docs/55 §2.9 写回 docs/12（或 docs/16）**，并把 P50/P95、口径、样本对数、误差项、rec.wav 与 json 的 SHA-256 一起落文档（target/ 不入库，哈希才留得住）。

---

## 9. 本次交付物清单

| 文件 | 说明 |
|---|---|
| [tools/acoustic-latency.ps1](tools/acoustic-latency.ps1) | 量具本体（1482 行）：-SelfTest / -DryRun / -Run / -AnalyseWav |
| target/evidence/m1-device-p1/acoustic-selftest.log | 自检 + 两种 DryRun + 离线分析 的完整证据（269 行，SHA-256 7187c1a979dba22034dcf52d1f473051cd3c75b7b3b9d9cac937251e86401ae5） |
| target/evidence/m1-device-p1/selftest/*.wav | 自检合成录音（真值已知）+ 生成的激励 WAV |
| 本文 | 用法 / 摆位清单 / 自检证据 / 误差表 / 待办 |

> 工作区 git HEAD：903feea dirty=True（本文只描述脚本这一半；真机读数由现场那一半补齐。）
