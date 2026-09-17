package com.gotkicry.audiolink.service

/**
 * 省电白名单（FR-37 / docs/05 M5 交付物 2）。
 *
 * **为什么这是真需求**：长连接、采集、播放全活在**前台服务**里，而 Android 的电池优化会回收
 * 后台的前台服务 —— 用户把手机放一边听音频，系统可能在一段时间后把它优化掉，声音就断了。
 * 引导用户把本应用加入白名单，是这类应用的常规且必要的动作（也是 docs/04 指出的：自 API 34 起
 * `WIFI_MODE_FULL_HIGH_PERF` 被降级，"不要指望 WifiLock 对抗熄屏省电"）。
 *
 * **分层**（与 [com.gotkicry.audiolink.compliance.NoticesLoader] 同一立场）：
 * 本文件是**纯逻辑**（不认识 Android），Android 那一侧（PowerManager / Intent / 偏好存储）在
 * [PowerWhitelistProbe] 里。这样"什么情况下该引导、说什么、按什么顺序跳设置页"能在纯 JVM 上单测，
 * 而真机上只剩"薄壳有没有接对"这一件事。
 */

/**
 * 检测与引导的窄输入：把 Android 的读数拆成原始字段。
 *
 * 单测只需要这三个字段，不需要 Context、不需要 PowerManager、不需要真机。
 *
 * @param sdkInt 运行期 `Build.VERSION.SDK_INT`（**运行期**读数：不写成构建期常量，否则旧版本分支不可测）。
 * @param ignoringBatteryOptimizations `PowerManager.isIgnoringBatteryOptimizations(packageName)` 的结果。
 * @param promptDismissed 用户点过"不再提示"（持久化在偏好里；这里只收结果，不碰存储）。
 */
data class PowerWhitelistSnapshot(
    val sdkInt: Int,
    val ignoringBatteryOptimizations: Boolean,
    val promptDismissed: Boolean,
)

/** 面板的呈现档位 —— UI 据此选布局，而不是自己再判一遍（判断只在 [PowerWhitelistMapper] 里）。 */
enum class PowerWhitelistVariant {
    /** 已在白名单：只需一行状态，不占版面。 */
    Ok,

    /** 未加入且没被忽略过：给一条可操作的引导。 */
    Action,

    /** 未加入，但用户已经说过"别烦我"：低调呈现，**保留入口**。 */
    Quiet,

    /** 本机版本没有这个机制（API < 23）：如实说明，不给无效入口。 */
    Unsupported,
}

/**
 * 一条"跳系统设置页"的目标。
 *
 * 为什么不是光给一个 action 字符串：`ACTION_APPLICATION_DETAILS_SETTINGS` **必须带**
 * `package:` 数据才会命中本应用详情页；而 `ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS`
 * 打开的是**白名单列表**（不能带 data，否则可能无匹配）。这个差别是"兜底链能不能真的兜住"的关键，
 * 所以它必须进纯逻辑、被单测钉住。
 *
 * @param action `Intent` 的 action 字符串（**字面量**：单测里按字面量断言，不共享常量）。
 * @param needsPackageUri 是否需要附上本应用的 `package:` URI。
 */
data class PowerWhitelistAction(
    val action: String,
    val needsPackageUri: Boolean,
)

/**
 * 引导卡的内容（用户可能真的要照着做，所以标题与按钮文案在这里定死，UI 只渲染**零文案**）。
 *
 * @param title 卡片的标题句。
 * @param actionLabel 主按钮文案。
 * @param dismissLabel "不再提示"按钮的文案；`null` = 不给这个按钮（已经被忽略过的不再重复给）。
 */
data class PowerWhitelistPrompt(
    val title: String,
    val actionLabel: String,
    val dismissLabel: String?,
)

/**
 * 面板要渲染的一切。
 *
 * @param variant 呈现档位（UI 选布局用；判断不在 UI 里做）。
 * @param statusLabel 状态行（"已加入 / 未加入 / 本机不支持检测"）—— **任何档位下都如实给出**：
 *   用户排查"后台声音断了"时，第一眼要能看到白名单到底加没加。
 * @param detail 一句说明（为什么要加 / 为什么没法检测）。
 * @param runtimeNote 版本相关的补充说明；`null` = 本机版本不需要这句。
 * @param prompt 引导卡；`null` = 不弹引导。
 * @param actionLabel 常驻设置入口的按钮文案；`null` = 没有可跳的页面（已加入 / 本机不支持）。
 * @param actions 跳设置页的**尝试链**（依次试，前一个打不开就试下一个）。
 */
data class PowerWhitelistUiState(
    val variant: PowerWhitelistVariant,
    val statusLabel: String,
    val detail: String,
    val runtimeNote: String? = null,
    val prompt: PowerWhitelistPrompt? = null,
    val actionLabel: String? = null,
    val actions: List<PowerWhitelistAction> = emptyList(),
) {
    /** 是否展示引导卡（UI 用它决定要不要占版面）。 */
    val showsPrompt: Boolean get() = prompt != null

    /** 是否给得出一个能跳的设置页。 */
    val canOpenSettings: Boolean get() = actionLabel != null && actions.isNotEmpty()
}

/**
 * 窄输入 → 面板状态。**全是纯函数**：不碰 Android、不碰存储、不碰时间。
 */
object PowerWhitelistMapper {

    /**
     * 打开**白名单列表**让用户自己选（FR-37 指定的形态）。
     *
     * 与它成对的那条路是 `ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS`（直接弹出
     * "是否允许忽略电池优化？"的对话框）—— 本项目**刻意不用它**，两个理由：
     * 1. 它要求 `REQUEST_IGNORE_BATTERY_OPTIMIZATIONS` 权限，而该权限在 Google Play 上属敏感项
     *    （可用于绕过 Doze、后台常驻），有政策审核风险 —— 与本项目已挂的 [M5] 上架合规待办同源；
     * 2. 由系统列表页授权，用户看得见自己在给什么、也能看见别的应用在同一页的处境，
     *    比"点一下对话框就帮用户决定"更符合本项目的"如实呈现"立场。
     *
     * 所以本文件里**不存在**那条 action 的常量，单测也按字面量钉住"链里没有它"。
     */
    const val ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS =
        "android.settings.IGNORE_BATTERY_OPTIMIZATION_SETTINGS"

    /** 兜底：本应用详情页（部分厂商 ROM 没有上面的列表页，或它被改名/移除时会抛 ActivityNotFound）。 */
    const val ACTION_APPLICATION_DETAILS_SETTINGS =
        "android.settings.APPLICATION_DETAILS_SETTINGS"

    /**
     * 白名单检测（`isIgnoringBatteryOptimizations`）从 Android 6.0 / **API 23** 起可用。
     *
     * 本应用 minSdk = 26，所以这条分支在今天的真机上不可达；保留它**不是为了兼容旧机**，
     * 而是：版本矩阵是运行期读数（`Build.VERSION.SDK_INT`），"这个能力什么时候才有"属于映射表
     * 的内容，必须完整且可单测 —— 删掉它只会让"低版本怎么办"变成没人想过的空白。
     */
    const val DETECTION_MIN_API = 23

    /** Android 8.0 / **API 26** 起后台执行限制更严（本项目 minSdk 恰在此线）。 */
    const val BACKGROUND_LIMITS_API = 26

    /**
     * 跳设置页失败时给用户的话。
     *
     * 必须有：跳转在厂商 ROM 上有各种失败方式（页面被改名、被移除、被安全策略拦），
     * 静默什么都不发生是最糟的结果 —— 用户以为点了没反应，而问题出在系统那边。
     */
    const val OPEN_FAILED_NOTE =
        "没能打开系统设置页。可以手动进入「设置 → 电池 / 省电策略」，把 AudioLink 加入白名单（允许后台运行）。"

    /**
     * 依次尝试的设置页链：**先白名单列表，再本应用详情页**。
     *
     * 顺序是有理由的：列表页能一次看到"已加入白名单的应用"，是这个问题的主入口；
     * 详情页则是几乎所有 ROM 都保证存在的保守落点。
     */
    fun settingsChain(): List<PowerWhitelistAction> = listOf(
        PowerWhitelistAction(
            action = ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS,
            needsPackageUri = false,
        ),
        PowerWhitelistAction(
            action = ACTION_APPLICATION_DETAILS_SETTINGS,
            needsPackageUri = true,
        ),
    )

    fun map(snapshot: PowerWhitelistSnapshot): PowerWhitelistUiState {
        if (snapshot.sdkInt < DETECTION_MIN_API) {
            return unsupported()
        }
        if (snapshot.ignoringBatteryOptimizations) {
            return exempt()
        }
        return restricted(snapshot)
    }

    private fun exempt(): PowerWhitelistUiState = PowerWhitelistUiState(
        variant = PowerWhitelistVariant.Ok,
        statusLabel = "省电白名单：已加入",
        detail = "系统不会因省电策略限制本应用，后台播放与采集可以长期运行。",
    )

    private fun unsupported(): PowerWhitelistUiState = PowerWhitelistUiState(
        variant = PowerWhitelistVariant.Unsupported,
        statusLabel = "省电白名单：本机版本不支持检测",
        detail = "本机 Android 版本低于 6.0（API 23），没有省电白名单机制，后台运行时长由系统决定，" +
            "本应用无从检测也无从设置。",
    )

    private fun restricted(snapshot: PowerWhitelistSnapshot): PowerWhitelistUiState {
        val note = if (snapshot.sdkInt >= BACKGROUND_LIMITS_API) {
            "本机是 Android 8（API 26）或更高：后台执行限制更严，长期挂在后台更容易被系统回收。"
        } else {
            null
        }

        if (snapshot.promptDismissed) {
            return PowerWhitelistUiState(
                variant = PowerWhitelistVariant.Quiet,
                statusLabel = "省电白名单：未加入（你已选择不再提示）",
                detail = "后台播放与采集仍可能被系统的省电策略中断。想加入白名单时，随时可以打开系统设置页添加。",
                runtimeNote = note,
                actionLabel = "打开设置",
                actions = settingsChain(),
            )
        }

        return PowerWhitelistUiState(
            variant = PowerWhitelistVariant.Action,
            statusLabel = "省电白名单：未加入",
            detail = "系统的电池优化可能在一段时间后中断后台播放与采集 —— 把手机放一边听音频时最容易撞上。" +
                "加入白名单可以避免这类中断，不影响其它应用的省电。",
            runtimeNote = note,
            prompt = PowerWhitelistPrompt(
                title = "建议把 AudioLink 加入省电白名单",
                actionLabel = "打开省电白名单设置",
                dismissLabel = "不再提示",
            ),
            actionLabel = "打开省电白名单设置",
            actions = settingsChain(),
        )
    }
}