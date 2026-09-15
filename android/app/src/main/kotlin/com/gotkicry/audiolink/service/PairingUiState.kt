package com.gotkicry.audiolink.service

/**
 * 配对相关的 UI 状态（PIN + 对端列表）。
 *
 * 为什么单独建一个模型而不是直接把 FFI 的 `PeerView` 塞进 [PlaybackUiState]：
 * 1. `PeerView` 里嵌着一个 15+ 字段的 `TelemetryView`，把它拖进 UI 状态会让
 *    "PIN 有没有、对端受不受信"这两个最简单的判断也要先构造一整棵遥测树；
 * 2. 生成物（`audiolink_ffi.kt`）会随内核重新生成而变字段，UI 状态跟着抖；
 * 3. 映射逻辑必须能在**没有 `.so`、没有 Android** 的纯 JVM 上被单测钉住
 *    （同 [com.gotkicry.audiolink.diagnostics.ProtocolSelfTest] 的分层立场）。
 *
 * 所以：FFI → [PeerSnapshot]（窄输入）→ [PairingStateMapper] → [PeerUi]（UI 输出），三层各自可测。
 */

/**
 * 一个对端的 UI 快照。
 *
 * @param idShort 指纹短码（展示用，**不变**：信任判定的锚点）。
 * @param name 对端自报名（对端可随便写，**不参与信任判定**，所以不能只显示它）。
 * @param addr 对端地址 `ip:port`。
 * @param state 会话状态原始字面量（`idle`/`handshaking`/`streaming`/...，内核口径）。
 * @param stateLabel 会话状态的中文标签（未知状态原样透传，不吞）。
 * @param trusted 是否已在信任库中 —— 验收时判断"配对是否真的落盘"就看它。
 */
data class PeerUi(
    val idShort: String,
    val name: String,
    val addr: String,
    val state: String,
    val stateLabel: String,
    val trusted: Boolean,
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
    val trusted: Boolean,
)

/**
 * 配对面板的状态。
 *
 * @param pin 当前要展示给用户的 6 位配对 PIN；`null` = 当前没有配对在进行（UI 不占位）。
 * @param peers 已连接对端（可能为空）。
 * @param note 取 PIN / 对端列表失败时的人话原因；`null` = 没有错误。
 */
data class PairingUiState(
    val pin: String? = null,
    val peers: List<PeerUi> = emptyList(),
    val note: String? = null,
) {
    /** 是否有任何值得渲染的内容 —— UI 用它决定要不要占位。 */
    val hasAnything: Boolean get() = pin != null || peers.isNotEmpty()

    /**
     * 这个 PIN 是否**已经没有对应的活跃配对会话**（= 显示出来就会骗人）。
     *
     * 为什么需要这个判断（真机实测出来的）：内核侧 `PinGate` 是**每连接一个**
     * （`audiolink-engine/src/handshake.rs`：`Handshake.pin_gate`），连接一断 PIN 立刻作废；
     * 但 FFI 的事件缓存只在 `PairCompleted { ok: true }` 时清空，**对端断开不会清**
     * —— 于是 `displayedPin()` 在对端早就走了之后仍然返回旧 PIN。
     * 真机上出现过：所有 PC 侧进程都已退出，手机屏幕还挂着一个 6 位数字。
     *
     * 根因在 FFI 的事件处理（属 ffi 写域，已上报），外壳这里**不隐藏内核原值**，
     * 只把"它已经不作数了"如实标出来。
     *
     * 代理信号为什么用"还有没有未授信的对端会话"：配对待提交时，内核一定有一个
     * `handshaking` 的未授信会话（真机实测 `peers()` 此时会列出它，`trusted=false`）；
     * 会话没了就说明 PinGate 也没了。这是外壳能拿到的最接近 PinGate 生死的信号。
     */
    val pinIsStale: Boolean get() = PairingStateMapper.pinIsStale(pin, peers)
}

/**
 * [PeerSnapshot] → [PairingUiState] 的纯映射。
 *
 * 全部是纯函数：不碰 Android、不碰 FFI、不碰时间 —— 因此"PIN 有无两态"、
 * "对端没名字时显示什么"、"未知状态怎么显示"都能在 JVM 单测里逐条钉死。
 */
object PairingStateMapper {

    /**
     * PIN 是否已经没有对应的活跃配对会话（口径与理由见 [PairingUiState.pinIsStale]）。
     *
     * 抽成公开函数是为了让 [PairingUiState] 与 [PlaybackUiState] 用**同一份**规则 ——
     * 两处各写一遍 `none { !it.trusted }` 迟早会漂移。
     */
    fun pinIsStale(pin: String?, peers: List<PeerUi>): Boolean = pin != null && peers.none { !it.trusted }

    /**
     * PIN 归一化。
     *
     * 口径：只去首尾空白；**不做**"必须 6 位数字"的校验 —— 格式的权威在内核（`PinGate`），
     * 外壳再判一次只会在内核换了长度时把真 PIN 藏起来（"看不见的 PIN"比"格式不标准的 PIN"更糟）。
     * 空白串按"没有 PIN"处理，避免渲染出一个空的大字号框。
     */
    fun normalizePin(raw: String?): String? {
        val trimmed = raw?.trim().orEmpty()
        return trimmed.ifEmpty { null }
    }

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
     * 对端名是对端自报的、可留空；短指纹是信任判定的锚点，永远有值 —— 所以回落方向是
     * 「名字 → 短码」，而不是显示一个空行。
     */
    fun displayName(snapshot: PeerSnapshot): String =
        snapshot.name.trim().ifEmpty { snapshot.idShort.ifEmpty { "（未知对端）" } }

    /** 对端列表映射（保持内核给出的顺序）。 */
    fun peers(snapshots: List<PeerSnapshot>): List<PeerUi> = snapshots.map { snapshot ->
        PeerUi(
            idShort = snapshot.idShort,
            name = displayName(snapshot),
            addr = snapshot.addr,
            state = snapshot.state,
            stateLabel = stateLabel(snapshot.state),
            trusted = snapshot.trusted,
        )
    }

    /**
     * 总映射。
     *
     * @param pin 内核 `displayedPin()` 的原始返回值。
     * @param snapshots 内核 `peers()` 的原始返回值（取失败时传空表）。
     * @param error 取数失败时的人话原因（成功传 `null`）。
     */
    fun map(
        pin: String?,
        snapshots: List<PeerSnapshot>,
        error: String? = null,
    ): PairingUiState = PairingUiState(
        pin = normalizePin(pin),
        peers = peers(snapshots),
        note = error,
    )
}
