package com.gotkicry.audiolink.service

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * [LowLatencyDefaults] 的测试 —— 纯 JVM，不需要 Context / `.so` / 真机。
 *
 * 为什么这层值得单独钉：档位 → 水位的映射**在真机上无法直接验证**。
 * 20 ms 与 30 ms 的水位差在听感上极其微妙（网络抖动一大就淹没），真机听不出"水位是不是按档位设对了"，
 * 而设错的后果是延迟白降（帧长缩了但缓冲还在）或反过来白白爆音。所以判定必须在这里逐条钉死。
 *
 * **规矩（沿用 PowerWhitelistMapperTest 立的第 107 轮做法）**：断言里不引用被测实现的推导，
 * 帧数与毫秒数一律写**字面量**。用 `LowLatencyDefaults.queueTargetFor(true)` 去断言它自己，
 * 等于让实现自己给自己判卷 —— 哪天常量被改成 1440 测试照样全绿，而真机上低延迟档就不成立了。
 */
class LowLatencyDefaultsTest {

    // ---------- 默认档位 ----------

    @Test
    fun defaultIsStandardMode() {
        assertFalse(
            "开箱即低延迟会让升级悄悄改变老用户的带宽与耗电 —— 默认必须是标准档",
            LowLatencyDefaults.DEFAULT_LOW_LATENCY,
        )
    }

    // ---------- 档位 → 水位默认值 ----------

    @Test
    fun standardModeKeepsThirtyMillisecondWatermark() {
        // 30 ms × 48 kHz = 1440 帧。写死数字，不引用 DEFAULT_QUEUE_TARGET_FRAMES。
        assertEquals(
            "标准档必须保持既有的 30 ms 水位，否则等于偷偷改了所有人的现状",
            1_440,
            LowLatencyDefaults.queueTargetFor(lowLatency = false),
        )
    }

    @Test
    fun lowLatencyModeKeepsOneChunkOfOutputHeadroom() {
        assertEquals(
            "10 ms 拉取块需要 20 ms 输出目标，才能在队列清空之前补写",
            960,
            LowLatencyDefaults.queueTargetFor(lowLatency = true),
        )
    }

    @Test
    fun theTwoModesActuallyDiffer() {
        // 防"两个分支都返回同一个常量"这类复制粘贴错误：只断各自的绝对值还不够，
        // 因为有人可能把两处都改成 1440 而只看到"测试全绿"。
        assertNotEquals(
            "两档的水位若相同，低延迟档就名不副实（帧长变了、缓冲没变）",
            LowLatencyDefaults.queueTargetFor(lowLatency = false),
            LowLatencyDefaults.queueTargetFor(lowLatency = true),
        )
    }

    @Test
    fun lowLatencyWatermarkLeavesOneChunkAheadOfTheGate() {
        val standard = LowLatencyDefaults.queueTargetFor(lowLatency = false)
        val lowLatency = LowLatencyDefaults.queueTargetFor(lowLatency = true)

        assertEquals("低延迟目标比标准档少 10 ms", standard - 480, lowLatency)
        assertTrue("目标必须大于一块 10 ms 音频", lowLatency > 480)
    }

    // ---------- 切换开关时的重置值 ----------

    @Test
    fun switchResetMatchesTheModeDefault() {
        assertEquals(
            "用户切档时水位要重置成新档的默认值（这是「档位决定默认值」的唯一一次落地）",
            960,
            LowLatencyDefaults.queueTargetForSwitch(lowLatency = true),
        )
        assertEquals(
            "切回标准档同样要回到 30 ms，否则用户的滑杆会停在 20 ms 上，与档位名实不符",
            1_440,
            LowLatencyDefaults.queueTargetForSwitch(lowLatency = false),
        )
    }

    @Test
    fun userFreeChoiceIsRepresentable() {
        // 滑杆的合法取值域里必须能表达"不设目标"（0）—— 档位的默认值**不是**这个值，
        // 两者不能混（0 在滑杆上代表"满灌"，是 A/B 对照的一侧，与档位无关）。
        assertTrue("档位不该把水位默认值定成 0（那是满灌/Hold 档，不是延迟档）",
            LowLatencyDefaults.queueTargetFor(true) > 0)
        assertTrue("标准档同理，默认值必须是真实水位", LowLatencyDefaults.queueTargetFor(false) > 0)
    }

    // ---------- 帧长联动：协商帧长 → 水位 ----------

    // 这组测试的处境与上面那组**不同**：上面的档位是用户自己设的，设错了用户下次看设置页能发现；
    // 协商帧长是**对端**给的，用户看不见也改不了 —— 一旦算错，表现是「声音发闷/断续」或
    // 「延迟白降」，而两端都不报错（帧长不一致的经典症状）。所以这组必须把真值表逐格写死。

    @Test
    fun negotiatedTenMillisecondFrameWantsLowLatencyWatermark() {
        // 10 ms 帧需要留 10 ms 播放余量，目标水位为 20 ms。
        assertEquals(
            "对端用 10 ms 推流时，水位必须跟到 20 ms",
            960,
            LowLatencyDefaults.queueTargetForNegotiatedFrame(10),
        )
    }

    @Test
    fun negotiatedTwentyMillisecondFrameKeepsStandardWatermark() {
        // 20 ms x 48 kHz = 1440 帧（30 ms）。**刻意不等于 20 ms**：
        // 内核不按本端水位发帧，把缓冲收窄到 20 ms 只会让播放环开始欠载（听感断续）。
        assertEquals(
            "对端用 20 ms 推流时，水位保持标准档的 30 ms",
            1_440,
            LowLatencyDefaults.queueTargetForNegotiatedFrame(20),
        )
    }

    @Test
    fun illegalNegotiatedFramesHaveNoOpinion() {
        // 「没有意见」必须是可表达的一等结果（null），而不是硬塞一个 1440：
        // 内核今天只协商 10/20，但将来加了 60 ms 档，猜成「标准档」就是**算错**。
        // 0 尤其不能当成「没协商」——那是另一件事（见 PeerSnapshot 的字段注释）。
        for (illegal in listOf(0, 1, 15, 30, 40, 60, 255)) {
            assertNull(
                "不认识的帧长 " + illegal + " 必须没有意见，而不是被猜成一个水位",
                LowLatencyDefaults.queueTargetForNegotiatedFrame(illegal),
            )
        }
        assertNull("未协商（null）同样没有意见", LowLatencyDefaults.queueTargetForNegotiatedFrame(null))
    }

    // ---------- 帧长联动：该不该重置（真值表，逐格钉住）----------

    /**
     * 完整真值表 —— 旧协商帧长 x 新协商帧长 -> 是否该重置水位。
     *
     * 为什么写成一张表而不是几条散断言：这张表**就是接口**。
     * 「断流不重置」与「重复同值不重置」这两条如果只测其中一条，另一条改坏了照样全绿 ——
     * 而它们坑的恰恰是**手动滑杆**（用户拖了没用），真机上最难归因的一类问题。
     */
    @Test
    fun resetTruthTableIsPinned() {
        val table = listOf(
            Triple(null, 10, true),   // 开流协商确立 → 跟随 10 ms
            Triple(null, 20, true),   // 开流协商确立 → 跟随 20 ms
            Triple(null, null, false), // 还没开流：没有任何结果可跟
            Triple(10, 10, false),    // 幂等：peers() 每 500 ms 报同一个值
            Triple(20, 20, false),    // 同上
            Triple(10, 20, true),     // 对端换档或换发送端 → 跟随新值
            Triple(20, 10, true),     // 同上
            Triple(10, null, false),  // **断流不重置**：本档位最要紧的一条
            Triple(20, null, false),  // 同上
            Triple(10, 15, false),    // 非法新值不猜，也不能因为「值变了」就重置
            Triple(20, 0, false),     // 同上（0 是非法帧长，不是「没协商」）
        )
        for ((previous, next, expected) in table) {
            assertEquals(
                "协商帧长 " + previous + " → " + next + " 的重置判定错了",
                expected,
                LowLatencyDefaults.shouldResetForNegotiatedFrame(previous, next),
            )
        }
    }

    @Test
    fun disconnectNeverResetsTheWatermark() {
        // 单独再钉一次「断流不重置」，因为它与真值表里其它行的**代价**不同：
        // 真值表里判错的其它行顶多让水位慢一拍；这一行判错 = 每断一次流就把用户手动调好的
        // 水位按回默认值，而用户完全不知道是谁改的（症状就是滑杆「拖了没用」）。
        for (previous in listOf<Int?>(null, 10, 20, 15)) {
            assertFalse(
                "协商帧长从 " + previous + " 变回 null（断流）时绝不能动水位",
                LowLatencyDefaults.shouldResetForNegotiatedFrame(previous, null),
            )
        }
    }

    @Test
    fun repeatedSameValueIsIdempotent() {
        // 幂等的**可观察后果**：连读十拍同一个协商值，一次都不该重置。
        // 这条不是上面那两格的重复 —— 它钉的是「这个判断会被反复调用」这个使用方式，
        // 而 LowLatencyDefaults 自己并不知道调用频率。
        val resets = (1..10).map { LowLatencyDefaults.shouldResetForNegotiatedFrame(10, 10) }
        assertEquals("同值重复上报一次都不该重置", 0, resets.count { it })
        assertTrue(
            "但首次确立（null → 10）必须重置一次，否则跟随根本不生效",
            LowLatencyDefaults.shouldResetForNegotiatedFrame(null, 10),
        )
    }

    @Test
    fun resetDecisionAndMappedValueNeverDisagree() {
        // 两条规则之间的**一致性不变量**：凡判定要重置的，必须真的有一个值可以重置；
        // 凡没有值可映射的，就不能判成重置。分开写两条规则时最容易漏的正是这个缝 ——
        // 表现为「判定要重置，但重置成什么不知道」，服务侧只能静默跳过（水位不动，用户看不出错）。
        val candidates = listOf<Int?>(null, 0, 10, 15, 20, 30)
        for (previous in candidates) {
            for (next in candidates) {
                val should = LowLatencyDefaults.shouldResetForNegotiatedFrame(previous, next)
                val mapped = LowLatencyDefaults.queueTargetForNegotiatedFrame(next)
                if (should) {
                    assertNotNull(
                        "判定要重置（" + previous + " → " + next + "）却没有可用的水位值",
                        mapped,
                    )
                } else if (mapped == null) {
                    assertFalse(
                        "没有可用的水位值时绝不能判定重置（" + previous + " → " + next + "）",
                        should,
                    )
                }
            }
        }
    }
}
