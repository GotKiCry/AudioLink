package com.gotkicry.audiolink.service

/**
 * 对端列表的 UI 状态（内核 `peers()` 当前会话表的容器）。
 *
 * 为什么单独建一个模型而不是直接把 FFI 的 `PeerView` 塞进 [PlaybackUiState]：
 * 1. `PeerView` 里嵌着一个 15+ 字段的 `TelemetryView`，把它拖进 UI 状态会让
 *    「有没有对端、它在什么状态」这两个最简单的判断也要先构造一整棵遥测树；
 * 2. 生成物（`audiolink_ffi.kt`）会随内核重新生成而变字段，UI 状态跟着抖；
 * 3. 映射逻辑必须能在**没有 `.so`、没有 Android** 的纯 JVM 上被单测钉住
 *    （同 [com.gotkicry.audiolink.diagnostics.ProtocolSelfTest] 的分层立场）。
 *
 * 所以：FFI → [PeerSnapshot]（窄输入）→ [PeerStateMapper] → [PeerUi]（UI 输出），三层各自可测。
 */

/**
 * 一个对端的 UI 快照。
 *
 * @param idShort 指纹短码（展示用；完整指纹见 [idHex]）。
 * @param name 对端自报名（对端可留空，缺失时由 [PeerStateMapper.displayName] 回落到短码）。
 * @param addr 对端地址 `ip:port`。
 * @param state 会话状态原始字面量（`idle`/`handshaking`/`streaming`/...，内核口径）。
 * @param stateLabel 会话状态的中文标签（未知状态原样透传，不吞）。
 */
data class PeerUi(
    val idShort: String,
    val name: String,
    val addr: String,
    val state: String,
    val stateLabel: String,
    /**
     * 估算端到端延迟（us）；0 = 还没有遥测（未推流）。
     *
     * 为什么在 UiState 上补这三个字段（只加字段、不改任何行为）：
     * 产品真相要求首屏回答“在推什么、给谁、质量如何”，其中“质量如何”就是这三个读数 ——
     * 它们本来就在 FFI 的 PeerView.telemetry 里，但旧的 PeerSnapshot 映射把它们剥掉了，
     * UI 拿不到（UI 不许碰 FFI 类型，那是本文件类注释里定下的分层）。
     */
    val e2eLatencyUs: Long = 0L,
    /** 实际编码码率（bps）；0 = 还没有遥测。 */
    val bitrateBps: Long = 0L,
    /** 丢包率（百分比，0.0-100.0）。 */
    val lossPct: Double = 0.0,
    /**
     * 完整指纹 —— 调**按设备**的 FFI 时传的就是它。
     *
     * 为什么不传短码：内核的报错原文是「peer short id {short} matches more than one session;
     * pass the full idHex」。多设备时短码可能撞车，而按设备控制正是为多设备准备的。
     */
    val idHex: String = "",
    /**
     * 本机这一路的音量（千分点 0–2000；`null` = 用户没设过，等价 1000）。
     *
     * `null` 与「设成了 1000」是两件事：界面据此决定要不要打「已调整」标记（FFI 的 KDoc 明说）。
     */
    val localGain: UInt? = null,
    /** QUIC 往返延迟；0 表示尚未采样，不是端到端音频延迟。 */
    val rttUs: Long = 0L,
    /**
     * 本条流协商生效的 Opus 帧长（ms）；`null` = 未协商/未知（口径见 [PeerSnapshot.negotiatedFrameMs]）。
     *
     * 与 [AudioLinkService] 里那个**本端档位**（`lowLatency`）是两件事：那个管**发送方向**，
     * 这个说的是**接收方向**对端定了什么。诊断面板据此回答「跟随到底生效了没有」——
     * 帧长不一致的典型症状是听感发闷/断续而遥测全绿，只看本端档位看不出来。
     */
    val negotiatedFrameMs: Int? = null,
)

/**
 * 映射的**窄输入**：把 FFI `PeerView` 拆成原始字段。
 *
 * 这样单元测试不需要拉起 JNA/原生库，也不需要构造生成物里那棵遥测树；
 * 服务侧只做一次 `PeerView -> PeerSnapshot` 的字段拷贝。
 */
data class PeerSnapshot(
    val idShort: String,
    val name: String,
    val addr: String,
    val state: String,
    /** 估算端到端延迟（us）；0 = 无遥测。理由见 [PeerUi.e2eLatencyUs]。 */
    val e2eLatencyUs: Long = 0L,
    /** 实际编码码率（bps）；0 = 无遥测。 */
    val bitrateBps: Long = 0L,
    /** 丢包率（百分比 0.0-100.0）。 */
    val lossPct: Double = 0.0,
    /**
     * 完整指纹：按设备调 FFI 用的就是它（见 [PeerUi.idHex]）。
     * **必须留在末尾**：构造点里有按位置传参的调用，插在中间会把后面所有字段挪位。
     */
    val idHex: String = "",
    val rttUs: Long = 0L,
    /**
     * 本端作为**接收端**时，本条流协商生效的 Opus 帧长（ms）；**`null` = 未协商/未知**。
     *
     * 为什么在这一层就用可空（而不是拿 0 当哨兵）：「没有值」要**原样穿过纯逻辑层** ——
     * [LowLatencyDefaults.shouldResetForNegotiatedFrame] 的 `previous`/`next` 都用 null 表意，
     * 而「断流时变回 null」正是它必须区分的一条。让 0 兼职「没协商」，会把「0 ms 帧」这个
     * 不存在的档和「没协商」混成一件事 —— 而那两者恰恰必须分开。
     *
     * `Int?` 而非`UByte?`：本字段只做「等不等于 10 / 20」的比较，且要跨层传给纯逻辑对象 ——
     * 无符号类型在这里不带来任何表达能力（帧长不可能超过 255，0–255 在 Int 里没有歧义），
     * 只会给 JVM 单测添麻烦。窄化发生在**服务侧映射**那一行（离 FFI 最近的地方），
     * 那里用 `?.toInt()` 把 `UByte?` 落成 `Int?`。
     *
     * **末尾字段**：与 [idHex] 同样的约定 —— 前面几个字段有按位置传参的调用点，
     * 往后加字段不会挪位，插在中间会。
     */
    val negotiatedFrameMs: Int? = null,
)

/**
 * 对端列表 + 取数失败原因的不可变快照（FFI `peers()` 的一次查询产物）。
 *
 * 为什么把 `peers` 与 `note` 装进**同一个**不可变对象：它们是同一次调用的产物，
 * 必须同拍发布 —— 拆成两个字段分别赋值，会出现「列表已经更新、错误提示还挂着上一拍」
 * 这种自相矛盾的中间态，而读它的主线程无法分辨哪一半才是新的。
 *
 * @param peers 当前会话表（可能为空）。
 * @param note 取对端列表失败时的人话原因；`null` = 没有错误。
 */
data class PeerUiState(
    val peers: List<PeerUi> = emptyList(),
    val note: String? = null,
)

/**
 * [PeerSnapshot] → [PeerUi] 的纯映射。
 *
 * 全部是纯函数：不碰 Android、不碰 FFI、不碰时间 —— 因此「对端没名字时显示什么」、
 * 「未知状态怎么显示」都能在 JVM 单测里逐条钉死。
 */
object PeerStateMapper {

    /** 会话状态字面量 → 中文标签；未知值**原样返回**（不吞、不假装认识）。 */
    fun stateLabel(state: String): String = when (state.trim().lowercase()) {
        "idle" -> "空闲"
        "handshaking" -> "握手中"
        "streaming" -> "已连接"
        "degraded" -> "质量下降"
        "reconnecting" -> "重连中"
        "failed" -> "失败"
        else -> state
    }

    /**
     * 对端名缺失时的显示口径：回落到指纹短码。
     *
     * 对端名是对端自报的、可留空；指纹短码由本端从对端证书现算，永远有值 —— 所以回落方向是
     * 「名字 → 短码」，而不是显示一个空行。
     */
    fun displayName(snapshot: PeerSnapshot): String =
        snapshot.name.trim().ifEmpty { snapshot.idShort.ifEmpty { "（未知对端）" } }

    /**
     * 总映射：内核 `peers()` 的原始返回值 → UI 状态。
     *
     * @param snapshots 内核 `peers()` 的原始返回值（取失败时传空表）。
     */
    fun map(snapshots: List<PeerSnapshot>): PeerUiState = PeerUiState(peers = peers(snapshots))

    /** 对端列表映射（保持内核给出的顺序）。 */
    fun peers(snapshots: List<PeerSnapshot>): List<PeerUi> = snapshots.map { snapshot ->
        PeerUi(
            idShort = snapshot.idShort,
            idHex = snapshot.idHex,
            name = displayName(snapshot),
            addr = snapshot.addr,
            state = snapshot.state,
            stateLabel = stateLabel(snapshot.state),
            e2eLatencyUs = snapshot.e2eLatencyUs,
            rttUs = snapshot.rttUs,
            bitrateBps = snapshot.bitrateBps,
            lossPct = snapshot.lossPct,
            negotiatedFrameMs = snapshot.negotiatedFrameMs,
        )
    }
}