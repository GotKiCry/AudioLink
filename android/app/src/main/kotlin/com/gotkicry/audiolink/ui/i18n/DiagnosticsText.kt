package com.gotkicry.audiolink.ui.i18n

/**
 * 诊断面板的副标题：**内核报错时先说这件事**。
 *
 * 为什么值得单独一个函数：引擎起不来时，界面主路径上只会说一句"服务没能启动；请再试一次"，
 * 而**真正的原因（内核原文）只写在折叠的诊断区里** —— 用户不会展开一个看起来"没问题"的折叠区，
 * 于是真机上就只能看到"请再试一次"（2026-09 实测：实际报的是 UniFFI checksum mismatch，
 * 排查绕了好大一圈才看到）。
 *
 * 所以折叠态那句副标题要能自救：有报错就明说"展开看原因"。
 */
internal fun diagnosticsSummary(strings: UiStrings, engineError: String?): String =
    if (engineError.isNullOrBlank()) strings.diagnosticsSummary else strings.diagnosticsSummaryError
