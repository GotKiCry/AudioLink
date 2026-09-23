package com.gotkicry.audiolink.service

import com.gotkicry.audiolink.audio.LowLatencyPlayer

/**
 * 「低延迟档」的两条纯逻辑规则：默认值，以及**开关 → 播放水位默认值**的联动。
 *
 * ## 为什么单独抽一个对象
 * 与 [PowerWhitelistMapper] 同一立场：真正会出错的是**判断**，不是存取。
 * 而 [LowLatencyPreference] 要 `Context`（真机/仪器测试才能覆盖），把判断留在那里，
 * 「低延迟档的水位默认值该是多少」就只能靠真机试听来验证 —— 那是验证不了 20 ms 与 30 ms 差别的。
 * 抽出来之后这条规则在纯 JVM 上被逐条钉住（见 `LowLatencyDefaultsTest`）。
 *
 * ## 与手动水位滑杆的关系（**这是本档位最容易踩错的地方**）
 * `DiagnosticsDeck` 里已经有一支手动水位滑杆（`AudioLinkService.setQueueTargetFrames`），
 * 它允许用户在水位上任意取值（含 0 = 满灌）。于是同一件事有两个来源，必须定优先级：
 *
 *   **档位只决定「默认值 / 开关切换那一刻的重置值」，用户手动调节永远优先。**
 *
 * 理由：滑杆是**排障/调优工具**（A/B 对照、按现场网络质量找平衡点），档位是**一次性偏好**。
 * 若反过来让档位持续覆盖滑杆，用户刚拖到 60 ms 稳一稳，下一拍（服务每 500 ms 下发一次）就被
 * 按回 30 ms —— 滑杆等于坏了，而且症状是「拖了没用」，极难排查。
 * 所以档位**只在两个精确时刻**产生作用：
 *   1. 进程里还没人调过滑杆时，它就是初始默认值；
 *   2. 用户**切换开关**时（见 [queueTargetForSwitch]），把水位**重置**到新档的默认值 ——
 *      这是用户显式表达的意图，覆盖旧的手动值是正确的；之后手动调节依旧优先。
 *
 * 这样做还有一个副作用是好的：档位切换**立刻**影响水位（不必等引擎重启），
 * 而 10 ms 帧与 20 ms 输出目标配套：至少留一块未播数据，避免耗尽后才补写。
 *
 * ## 第三个来源：接收端的**协商帧长**（帧长联动）
 * 内核已实现帧长联动：本端作为**接收端**时，流的 Opus 帧长由发送端在 `OPEN_STREAM.codec_prefs` 决定、
 * 本端跟随（结果见 `PeerView.negotiatedFrameMs`）。所以水位还有第二个自动来源，
 * 判据与上面**完全同构**：协商帧长确立（或变化）时重置水位默认值，用户手动调节永远优先。
 * 三个来源谁在什么时候说话，就是本类的全部内容：
 *
 *   1. [queueTargetFor] —— 本端**发送方向**档位，服务创建时的初始默认值；
 *   2. [queueTargetForSwitch] —— 用户切档那一刻的重置；
 *   3. [queueTargetForNegotiatedFrame] —— **接收方向**协商结果确立/变化那一刻的重置（幂等，见 [shouldResetForNegotiatedFrame]）。
 *
 * ⚠️ 1 与 3 会在同一条流上先后说话（本端开着低延迟档当发送端，同时作为接收端跟随对端的 20 ms），
 * 这是**设计的本意**而非冲突：档位管发送方向，协商管接收方向，两者本来就该能各说各的。
 */
object LowLatencyDefaults {

    /**
     * 档位默认值：**关**（标准档 = 20 ms Opus 帧 + 30 ms 水位）。
     *
     * 开着的默认值就是「行为与现状完全一致」—— 这是升级不改变老用户听感的承诺，不能是 true。
     */
    const val DEFAULT_LOW_LATENCY = false

    /**
     * 按档位取播放水位的**默认值**（帧）。
     *
     * 低延迟档 = [LowLatencyPlayer.LOW_LATENCY_QUEUE_TARGET_FRAMES]（960 帧 = 20 ms）：
     * 与 10 ms 帧长配套 —— 帧长缩了而水位不动，等于延迟只降了编码侧的那 10 ms，
     * 缓冲里的 30 ms 还在等着，用户听到的改善会远小于预期。
     *
     * 标准档 = [LowLatencyPlayer.DEFAULT_QUEUE_TARGET_FRAMES]（1440 帧 = 30 ms）：**原值不动**。
     */
    fun queueTargetFor(lowLatency: Boolean): Int = if (lowLatency) {
        LowLatencyPlayer.LOW_LATENCY_QUEUE_TARGET_FRAMES
    } else {
        LowLatencyPlayer.DEFAULT_QUEUE_TARGET_FRAMES
    }

    /**
     * 开关**切换**时的水位重置值。
     *
     * 今天与 [queueTargetFor] 是同一个函数，为什么还要留一个入口：两者的**触发时机**不同，
     * 而时机正是这个档位全部语义的所在。将来若要「切档时也保留用户的手动值」，改动点就该落在
     * 这里（那时它才需要读当前水位），而不是散落在 UI 的 onClick 里。
     * 名字比注释更难改错 —— 先把时机的名字占住。
     */
    fun queueTargetForSwitch(lowLatency: Boolean): Int = queueTargetFor(lowLatency)

    // -----------------------------------------------------------------------
    // 帧长联动（本端作为**接收端**时）
    // -----------------------------------------------------------------------

    /**
     * 水位**该跟随哪一档帧长** —— 本条流生效的帧长（ms），`null` = 未知/未协商。
     *
     * 自内核落地「帧长联动」后，**接收端**的 Opus 帧长不再由本端开关决定：
     * 它由发送端在 `OPEN_STREAM.codec_prefs` 里报出、本端跟随（协商结果经
     * `PeerView.negotiatedFrameMs` 给到壳侧）。本端开关从此只决定**发送方向**的帧长。
     *
     * 于是水位多了一个来源，判据与手动滑杆那套**完全同构**（见类注释）：
     *
     *   **协商帧长也只决定「默认值 / 协商确立（或变化）那一刻的重置值」，用户手动调节永远优先。**
     *
     * 为什么不跟 [queueTargetFor] 合并成一个函数：那个读的是**本端档位**（一个持久化布尔），
     * 这个读的是**对端协商结果**（一个可能为 null 的 u8）。两者在同一秒里可以给出相反答案
     * （本端开着低延迟档当发送端，同时作为接收端跟随了对端的 20 ms），
     * 合并就等于把「谁说了算」这件事藏进 if 里 —— 而它正是这条链路最容易错的地方。
     */
    fun queueTargetForNegotiatedFrame(frameMs: Int?): Int? = when (frameMs) {
        // 10 ms 帧 ⇄ 20 ms 目标：留一块 10 ms 输出余量（理由同 queueTargetFor）。
        FRAME_MS_LOW_LATENCY -> LowLatencyPlayer.LOW_LATENCY_QUEUE_TARGET_FRAMES
        // 20 ms 帧 ⇄ 30 ms 水位：沿用标准档水位，**刻意不跟着缩成 20 ms** ——
        // 内核不按本端水位发帧，把缓冲收窄到 20 ms 只会让播放环开始欠载（听感是断续）。
        FRAME_MS_STANDARD -> LowLatencyPlayer.DEFAULT_QUEUE_TARGET_FRAMES
        // 其余值（含 null / 0 / 15 / 30）**防御性忽略**：内核今天只协商 10 与 20，
        // 但将来加了 60 ms 档，这里硬塞一个 1440 会是把「不认识」当成「标准档」—— 那是猜。
        // 返回 null = 「没有意见」，调用方保持现状（水位不动，见 [shouldResetForNegotiatedFrame]）。
        else -> null
    }

    /**
     * 协商帧长变化时，水位**要不要重置**。
     *
     * 非空值的完整真值表（逐行钉在 `LowLatencyDefaultsTest.resetTruthTableIsPinned`）：
     *
     * | 旧 → 新        | 结果        | 为什么 |
     * |---------------|------------|--------|
     * | null → 10     | **重置 960** | 开流协商确立，这是「默认值规则」在接收方向的唯一一次落地 |
     * | null → 20     | **重置 1440**| 同上（值与「本端没开低延迟档」相同，但语义不同：这次是**对端**说的） |
     * | null → null   | 不重置      | 还没开流，没有任何协商结果可跟 |
     * | 10 → 10       | 不重置      | 幂等：同值重复上报（peers() 每 500 ms 一拍）不能反复把水位按回默认值 |
     * | 10 → 20       | **重置 1440**| 对端切了档或换了发送端，跟随新值 |
     * | 20 → 10       | **重置 960** | 同上 |
     * | *  → null     | 不重置      | **最要紧的一条**：断流时 negotiatedFrameMs 会变回 null，
     * |               |             | 此时把水位按回默认档是**替用户改了一个没人要求改的值** ——
     * |               |             | 用户手动拖到 60 ms 稳一稳、对面一断就被按回 30 ms，症状是「拖了没用」（同手动滑杆的坑） |
     * | 任意 → 非法(0/15/30…) | 不重置 | 不认识的值不猜（理由见 [queueTargetForNegotiatedFrame]） |
     *
     * 为什么不写成「返回值 + 调用方判断」：**幂等性要靠旧值**。每 500 ms 一拍都会重新读到同一个
     * negotiatedFrameMs，只判「新值合法」会让水位每拍被重置一次 —— 手动滑杆随即失效
     * （用户拖完不到 500 ms 就被覆盖）。所以这份判断必须同时看到新旧两个值，它就该住在这里。
     *
     * @param previous 上一拍见到的协商帧长；非法值请由调用方先归一为 null（见 [queueTargetForNegotiatedFrame]）。
     * @param next 这一拍读到的协商帧长（`PeerView.negotiatedFrameMs`）。
     */
    fun shouldResetForNegotiatedFrame(previous: Int?, next: Int?): Boolean {
        // 不认识新值 → 没有意见，保持现状。这一步必须在新旧比较**之前**：
        // 否则「10 → 15」会因为「值变了」被判成重置，而重置成什么我们其实不知道。
        if (queueTargetForNegotiatedFrame(next) == null) return false
        // 没协商过 → 有结果了，且本轮还没为它重置过 —— 唯一一次「开流即跟随」。
        if (previous == null) return true
        // 协商结果变了 → 跟随；同值 → 不重复重置（幂等）。
        return previous != next
    }

    /** 10 ms 帧（内核 `m1_low_delay_tight()`）。 */
    const val FRAME_MS_LOW_LATENCY = 10

    /** 20 ms 帧（内核默认档）。 */
    const val FRAME_MS_STANDARD = 20
}
