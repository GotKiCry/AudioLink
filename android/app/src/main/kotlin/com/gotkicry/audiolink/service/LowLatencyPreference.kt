package com.gotkicry.audiolink.service

import android.content.Context

/**
 * 「低延迟档」的持久化（10 ms Opus 帧 + 10 ms 播放水位）。
 *
 * **刻意贴着 [PowerWhitelistProbe] 的写法**：同为服务层设置、同用 SharedPreferences、
 * 同是「读一眼就知道没有第二个判断藏在里面」的薄壳。判断逻辑在 [LowLatencyDefaults]（纯 JVM 可单测），
 * 本文件只负责「存取一个布尔」。
 *
 * ## 为什么用一个新文件而不是挂进 PowerWhitelistProbe
 * 省电白名单那条链是 FR-37 的独立功能（[PowerWhitelistMapper] / [PowerWhitelistProbe] /
 * `PowerWhitelistDeck` 三层），把一个**音频链路**的档位塞进去，会让「白名单」这个小域承载
 * 两个不相干的概念 —— 将来扒白名单时会顺手带走档位。**两者共用 [PREFS_NAME] 一个文件**，
 * 因为都属于「服务层的设备级偏好」；分文件反而制造两份需要同步清理的存储。
 *
 * ## 生效时机（契约，与 desktop 端已拍定的做法对齐）
 * **只读一次、下次启动引擎生效**。切换开关时**不**重启引擎/前台服务 —— 重启会打断正在跑的
 * 播放与采集（用户的音频会断一下），为了「立即生效」付这个代价不值得。UI 文案必须写明这一点。
 */
object LowLatencyPreference {

    /**
     * 偏好文件名：**与 [PowerWhitelistProbe.PREFS_NAME] 同款**（同一个字面量）。
     *
     * 共用一份存储的理由：两者都是服务层的设备级偏好，分开写两个文件名只会让「清数据」要清两处。
     * 这里用 `const val` 再声明一次而不是引用那个 `private` 常量 —— 本项目的偏好名是**跨文件契约**，
     * 写死字面量能让「改了一处、另一处悄悄还在」在编译期就被发现（两处都会指向新名字，但值必须一致）。
     */
    const val PREFS_NAME = "audiolink_power"

    /**
     * 低延迟档的键名（契约：`low_latency`，默认 false = 标准档）。
     *
     * 与 desktop 端的键名（`lowLatency`）大小写风格不同：那边是 JSON 的驼峰，这边沿用
     * 本文件既有键的 snake_case（见 [PowerWhitelistProbe] 的 `whitelist_prompt_dismissed`）。
     * **刻意不统一** —— key 只在各自平台的存储内部使用，不存在互通需求。
     */
    const val KEY_LOW_LATENCY = "low_latency"

    /**
     * 读档位开关。键缺失 / 读失败一律回 [LowLatencyDefaults.DEFAULT_LOW_LATENCY]（= false 标准档）。
     *
     * 为什么 `runCatching`：SharedPreferences 在存储损坏时可能直接抛（而不只是返回默认值）。
     * 设置读不出来**不该带走音频链路** —— 读失败按默认档处理至少可预期（与 ThemePreference.read 同一立场）。
     */
    fun isEnabled(context: Context): Boolean = runCatching {
        preferences(context).getBoolean(KEY_LOW_LATENCY, LowLatencyDefaults.DEFAULT_LOW_LATENCY)
    }.getOrDefault(LowLatencyDefaults.DEFAULT_LOW_LATENCY)

    /**
     * 写档位开关。
     *
     * 用 `apply()`（异步落盘）而不是 `commit()`：写发生在主线程上用户点开关的那一刻，
     * `commit()` 的同步磁盘 IO 会卡首帧；而这里**没有「写完立刻要读到」的需求** ——
     * 值真正被消费是在下一次引擎启动（本次会话内的即时效果由 AudioLinkService 的
     * `applyLowLatencyDefault` 直接改内存里的水位默认值覆盖）。
     */
    fun setEnabled(context: Context, enabled: Boolean) {
        runCatching {
            preferences(context).edit().putBoolean(KEY_LOW_LATENCY, enabled).apply()
        }
    }

    private fun preferences(context: Context) =
        context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
}
