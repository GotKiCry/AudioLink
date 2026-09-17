package com.gotkicry.audiolink.service

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.PowerManager

/**
 * 省电白名单的 **Android 薄壳**（FR-37）。
 *
 * 这一层刻意只有一个职责：**把 Android 的读数与动作接给纯逻辑**。所有判断与文案都在
 * [PowerWhitelistMapper] 里（那一侧能在纯 JVM 上单测）；本文件里的东西需要真机/仪器测试才能覆盖，
 * 所以它必须足够薄 —— 薄到"读一眼就知道没有第二个判断藏在里面"。
 *
 * 真机项（未验证，见交付说明）：本机 SystemUI/厂商 ROM 上这些 Intent 的实际落点、
 * PowerManager 的读数刷新时机。
 */
object PowerWhitelistProbe {

    /** 偏好文件名与键：只存"用户说过别再提示"这一个布尔。 */
    private const val PREFS_NAME = "audiolink_power"
    private const val KEY_PROMPT_DISMISSED = "whitelist_prompt_dismissed"

    /**
     * 当前快照（读一次系统状态 + 偏好）。
     *
     * 为什么要"每次重新读"而不是缓存：用户很可能刚在系统设置页里改完再返回本应用，
     * 那一趟回来必须看到新状态 —— 缓存会让界面继续显示旧结论（而用户以为自己已经设置好了）。
     */
    fun snapshot(context: Context): PowerWhitelistSnapshot = PowerWhitelistSnapshot(
        sdkInt = Build.VERSION.SDK_INT,
        ignoringBatteryOptimizations = isIgnoringBatteryOptimizations(context),
        promptDismissed = isPromptDismissed(context),
    )

    /**
     * 本应用是否已在省电白名单里。
     *
     * `PowerManager.isIgnoringBatteryOptimizations` 是 API 23 起才有的 API，所以低版本直接返回
     * `false` —— 但低版本的"该怎么办"由 [PowerWhitelistMapper] 判（那里给的是"本机不支持检测"），
     * 这里只负责"读不出来就别瞎猜"。
     */
    private fun isIgnoringBatteryOptimizations(context: Context): Boolean {
        if (Build.VERSION.SDK_INT < PowerWhitelistMapper.DETECTION_MIN_API) {
            return false
        }
        val power = context.getSystemService(Context.POWER_SERVICE) as? PowerManager ?: return false
        return power.isIgnoringBatteryOptimizations(context.packageName)
    }

    fun isPromptDismissed(context: Context): Boolean =
        preferences(context).getBoolean(KEY_PROMPT_DISMISSED, false)

    fun markPromptDismissed(context: Context) {
        preferences(context).edit().putBoolean(KEY_PROMPT_DISMISSED, true).apply()
    }

    /**
     * 只存一个布尔，所以用 SharedPreferences 而不是 DataStore。
     *
     * 说明：项目依赖里声明了 `androidx.datastore:datastore-preferences`，但当前全仓**没有使用**
     * （grep 零命中）；为一个"别再提示我"的开关引入 DataStore 的协程/Flow 机制不划算。
     * 将来设置项多起来了再统一，届时连这个键一起搬。
     */
    private fun preferences(context: Context) =
        context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)

    /**
     * 依次尝试 [PowerWhitelistMapper.settingsChain] 里的设置页，成功一个就返回。
     *
     * 为什么要有链：部分厂商 ROM 没有白名单列表页（或改了名、被安全策略拦），
     * 直接跳会抛 [ActivityNotFoundException]。没有兜底就等于"点了没反应"。
     *
     * @return 是否成功拉起了某一个设置页；`false` 时调用方必须显示
     *   [PowerWhitelistMapper.OPEN_FAILED_NOTE]（含手动路径），不能静默。
     */
    fun openSettings(context: Context, chain: List<PowerWhitelistAction>): Boolean {
        for (target in chain) {
            val intent = Intent(target.action).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            if (target.needsPackageUri) {
                // 应用详情页必须带 package: 数据才会命中本应用，否则同样可能无匹配。
                intent.data = Uri.parse("package:" + context.packageName)
            }
            try {
                context.startActivity(intent)
                return true
            } catch (missing: ActivityNotFoundException) {
                continue
            } catch (denied: SecurityException) {
                continue
            }
        }
        return false
    }
}
