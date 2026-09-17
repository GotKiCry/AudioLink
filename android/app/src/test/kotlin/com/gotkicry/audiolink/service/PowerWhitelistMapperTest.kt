package com.gotkicry.audiolink.service

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * [PowerWhitelistMapper] 的测试 —— 全部在纯 JVM 上跑（不需要 Context、不需要 `.so`、不需要真机）。
 *
 * 这一层值得单独钉的理由：白名单这件事在真机上只有三种"看起来一样"的结果 ——
 * 已加入、未加入但没提示、以及提示了却跳不到设置页。三种都表现为"用户什么都没看到"，
 * 而处置完全不同。真机测不出这三种的差别，所以判定必须在这里逐条钉死。
 *
 * **规矩（第 107 轮立的）**：断言里**不引用被测实现的常量与推导** —— 版本号与 Intent action
 * 一律写字面量。用 `PowerWhitelistMapper.DETECTION_MIN_API` 写断言，等于让阈值自己给自己判卷：
 * 哪天它被改成 30，测试照样全绿，而真机上 API 24 的机器会突然显示"本机不支持检测"。
 */
class PowerWhitelistMapperTest {

    private fun snapshot(sdkInt: Int, ignoring: Boolean, dismissed: Boolean = false) =
        PowerWhitelistSnapshot(
            sdkInt = sdkInt,
            ignoringBatteryOptimizations = ignoring,
            promptDismissed = dismissed,
        )

    // ---------- 检测结果 → 该不该引导 ----------

    @Test
    fun exemptDeviceReportsStatusWithoutPrompt() {
        val state = PowerWhitelistMapper.map(snapshot(sdkInt = 34, ignoring = true))

        assertTrue("已在白名单就必须如实说出来（排查后台被杀时第一眼要看它）", state.statusLabel.contains("已加入"))
        assertFalse("已经加进白名单了还弹引导就是在浪费用户的注意力", state.showsPrompt)
        assertNull(state.prompt)
        assertFalse("没有可做的动作时不许给按钮", state.canOpenSettings)
        assertTrue("没有可跳的页面时动作链必须为空，否则 UI 会画出一个点了没反应的按钮", state.actions.isEmpty())
    }

    @Test
    fun restrictedDevicePromptsWithOperableEntry() {
        val state = PowerWhitelistMapper.map(snapshot(sdkInt = 34, ignoring = false))

        assertEquals(PowerWhitelistVariant.Action, state.variant)
        assertTrue("未加入白名单时该引导", state.showsPrompt)
        assertNotNull(state.prompt)
        assertTrue("引导必须给得出文案与动作（否则就是一条看了也做不了的提示）", state.prompt!!.actionLabel.isNotBlank())
        assertTrue(state.canOpenSettings)
        assertEquals("先白名单列表、再应用详情页，两条兜底", 2, state.actions.size)
        assertTrue("状态行仍要如实说未加入", state.statusLabel.contains("未加入"))
    }

    @Test
    fun promptWordingDoesNotThreatenTheUser() {
        val state = PowerWhitelistMapper.map(snapshot(sdkInt = 34, ignoring = false))
        val detail = state.detail

        assertTrue("说法要留余地：\"可能中断\"而不是\"一定中断\"", detail.contains("可能"))
        assertFalse("不许写成不设置就用不了（本应用没设置白名单也能跑，只是后台更容易被打断）", detail.contains("无法使用"))
        assertFalse("同上，换个说法也不行", detail.contains("不能使用"))
        assertFalse("不许拿\"必须\"压用户", state.prompt!!.title.contains("必须"))
    }

    @Test
    fun dismissedPromptStopsNaggingButKeepsTheEntry() {
        val state = PowerWhitelistMapper.map(snapshot(sdkInt = 34, ignoring = false, dismissed = true))

        assertEquals(PowerWhitelistVariant.Quiet, state.variant)
        assertFalse("用户点过\"不再提示\"之后不许再弹引导卡", state.showsPrompt)
        assertNull(state.prompt)
        assertTrue("但入口必须留着 —— 忽略≠拆掉动作，否则用户想加的时候无从下手", state.canOpenSettings)
        assertTrue("状态行仍要说清是\"已选择不再提示\"而不是\"已加入\"", state.statusLabel.contains("不再提示"))
        assertFalse("状态行不许让用户以为已经加好了", state.statusLabel.contains("已加入"))
    }

    @Test
    fun oldAndroidWithoutTheDetectionApiIsReportedAsUnsupported() {
        val state = PowerWhitelistMapper.map(snapshot(sdkInt = 22, ignoring = false))

        assertEquals(PowerWhitelistVariant.Unsupported, state.variant)
        assertFalse("读不出检测结果时不许猜（更不许假装未加入）", state.showsPrompt)
        assertFalse("跳过去也没有那个开关，给按钮就是骗人", state.canOpenSettings)
        assertTrue(state.actions.isEmpty())
        assertTrue("要如实说清\"本机不支持\"", state.statusLabel.contains("不支持"))
    }

    @Test
    fun detectionIsAvailableFromApi23() {
        // 边界另一侧：22 不可检测（见上一条），23 起可检测 —— 两个数字都是字面量。
        val state = PowerWhitelistMapper.map(snapshot(sdkInt = 23, ignoring = false))

        assertEquals(PowerWhitelistVariant.Action, state.variant)
        assertTrue("API 23 引入 isIgnoringBatteryOptimizations，这台机器必须有引导", state.showsPrompt)
    }

    // ---------- 版本差异 → 文案 ----------

    @Test
    fun backgroundLimitNoteAppearsFromApi26Only() {
        val before = PowerWhitelistMapper.map(snapshot(sdkInt = 25, ignoring = false))
        val at = PowerWhitelistMapper.map(snapshot(sdkInt = 26, ignoring = false))
        val after = PowerWhitelistMapper.map(snapshot(sdkInt = 34, ignoring = false))

        assertNull("API 26 之前不该出现后台限制更严这句（阈值写错时这条会红）", before.runtimeNote)
        val noteFrom26 = at.runtimeNote
        assertNotNull(noteFrom26)
        assertNotNull(after.runtimeNote)
        // 本应用 minSdk 就是 26，所以这句在真机上必然出现 —— 它说的是"限制更严"，不是"会坏"。
        assertTrue(noteFrom26?.contains("更严") == true)
    }

    // ---------- 引导的开关只在一处 ----------

    @Test
    fun promptOnlyAppearsWhenTheDeviceIsRestrictedAndNotDismissed() {
        val matrix = listOf(
            Triple(34, true, false) to false,
            Triple(34, false, false) to true,
            Triple(34, false, true) to false,
            Triple(26, true, true) to false,
            Triple(22, false, false) to false,
        )

        matrix.forEach { (input, expected) ->
            val (sdkInt, ignoring, dismissed) = input
            val state = PowerWhitelistMapper.map(snapshot(sdkInt, ignoring, dismissed))
            assertEquals(
                "sdkInt=" + sdkInt + " ignoring=" + ignoring + " dismissed=" + dismissed + " 的引导判定",
                expected,
                state.showsPrompt,
            )
        }
    }

    // ---------- 跳设置页的动作链 ----------

    @Test
    fun settingsChainStartsWithTheWhitelistListPage() {
        val first = PowerWhitelistMapper.settingsChain().first()

        assertEquals(
            "首选必须是白名单列表页（FR-37 指定的形态；字面量断言，防止实现被换成别的 action）",
            "android.settings.IGNORE_BATTERY_OPTIMIZATION_SETTINGS",
            first.action,
        )
        assertFalse("列表页不带 data —— 带了反而可能无匹配", first.needsPackageUri)
    }

    @Test
    fun settingsChainFallsBackToThisAppDetailsPage() {
        val second = PowerWhitelistMapper.settingsChain()[1]

        assertEquals(
            "兜底落点：应用详情页（几乎所有 ROM 都有）",
            "android.settings.APPLICATION_DETAILS_SETTINGS",
            second.action,
        )
        assertTrue("详情页**必须**带 package: 数据才会命中本应用", second.needsPackageUri)
    }

    @Test
    fun requestIgnoreBatteryOptimizationsIsNeverUsed() {
        // 政策取舍的钉子：ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS 需要
        // REQUEST_IGNORE_BATTERY_OPTIMIZATIONS 权限（Play 敏感项），本项目刻意不用它。
        // 这条断言把"任何分支都不许出现它"钉死 —— 包括将来新加的分支。
        val forbidden = "android.settings.REQUEST_IGNORE_BATTERY_OPTIMIZATIONS"
        val inputs = listOf(
            Triple(34, true, false),
            Triple(34, false, false),
            Triple(34, false, true),
            Triple(22, false, false),
            Triple(26, false, false),
        )

        inputs.forEach { (sdkInt, ignoring, dismissed) ->
            val chain = PowerWhitelistMapper.map(snapshot(sdkInt, ignoring, dismissed)).actions
            assertTrue(
                "sdkInt=" + sdkInt + " 的动作链里出现了受限 action：" + chain,
                chain.none { it.action == forbidden },
            )
            // 单测里也把"链长"钉住：偷偷加第三条同样是行为变化，必须显式确认。
            assertTrue("动作链最多两条（白名单列表 + 应用详情页）", chain.size <= 2)
        }
    }

    // ---------- 跳转失败的说法 ----------

    @Test
    fun openFailureNotePointsToTheManualPath() {
        val note = PowerWhitelistMapper.OPEN_FAILED_NOTE

        assertTrue("跳不动时必须说话，不能静默", note.isNotBlank())
        assertTrue("而且要给手动路径（用户自己去设置里找），而不是只说一句失败", note.contains("设置"))
    }
}
