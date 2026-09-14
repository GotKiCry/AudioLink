package com.gotkicry.audiolink.diagnostics

import com.gotkicry.audiolink.core.FfiException
import com.gotkicry.audiolink.core.protocolSelfTest
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * 内核协议层自检的结果（契约 §8 的 `protocolSelfTest()`，返回 `…RESULT: PASS (5/5)` 或失败详情）。
 *
 * 为什么要把它单独建模成三态而不是一个 Boolean：真机验收时"连不上"可能是**三种完全不同**的原因，
 * 三态让它们在一眼之间被分开：
 * - [Passed]：内核在真机上活着、§12 三组 golden vectors 逐字节正确 → 问题在**网络/配对**层；
 * - [Rejected]：内核明确拒绝（带 §11 错误码）→ 问题在**内核逻辑**；
 * - [Unavailable]：`.so` 没加载 / ABI 不匹配 / JNA 崩 → 问题在**打包或 ABI** 层。
 * 没有这个切分，人就只能拿着"没声音"同时怀疑三层。
 */
sealed interface SelfTestResult {

    /** 自检通过。[summary] 是内核返回的原始摘要（原样显示，不加工）。 */
    data class Passed(val summary: String) : SelfTestResult

    /**
     * 内核明确拒绝：`code` = §11 错误码数值，[shortName] = 错误码短名（如 `NOT_PAIRED`），
     * [context] = 出错细节。三个都显示出来 —— 吞成"自检失败"等于把线索丢了。
     */
    data class Rejected(val code: UShort, val shortName: String, val context: String) : SelfTestResult

    /** 压根没跑起来（原生库/依赖层问题）。[reason] 是人话原因。 */
    data class Unavailable(val reason: String) : SelfTestResult
}

/**
 * 协议自检入口。
 *
 * 两个刻意的设计：
 * 1. **不依赖引擎、不依赖服务**：它验的是协议编解码层（golden vectors），不是会话状态，
 *    所以服务没起、没连接、甚至没开网都能跑 —— 这使它成为真机排障的**第一个**可执行动作。
 * 2. **调用与结果映射分离**：[run] 负责在 IO 线程发起真实调用，[map] 是纯函数，
 *    因此"异常怎么变成人话"这件事在**没有 Android、没有 `.so`** 的 JVM 上就能被单测钉住。
 */
object ProtocolSelfTest {

    /**
     * 执行自检（可在任何线程调用；内部切到 IO）。
     *
     * 之所以必须在后台线程：第一次调用会触发 JNA `Native.load` 加载体内的 `libaudiolink_ffi.so`，
     * 并在内核侧跑完全部 golden vectors —— 这两件事都不该发生在主线程上。
     */
    suspend fun run(): SelfTestResult = withContext(Dispatchers.IO) {
        map { protocolSelfTest() }
    }

    /**
     * 把"一次调用"映射成可显示结果（纯逻辑，JVM 单测覆盖）。
     *
     * 异常分级的理由：
     * - `UnsatisfiedLinkError` 是 **Error 不是 Exception**（JNA 加载 `.so` 失败的形态），
     *   漏掉它会让整个 UI 崩掉，而它恰恰是"APK 里没打进本机 ABI"这类最容易发生、也最容易修的问题；
     * - 其余统一兜底并带上异常类型名 —— 宁可显示 `InternalException: …`，也不要显示"自检失败"。
     */
    internal fun map(call: () -> String): SelfTestResult = try {
        SelfTestResult.Passed(call())
    } catch (e: FfiException.Failure) {
        SelfTestResult.Rejected(
            code = e.code,
            shortName = e.messageText,
            context = e.context,
        )
    } catch (e: UnsatisfiedLinkError) {
        SelfTestResult.Unavailable(
            "原生库未加载（libaudiolink_ffi.so 缺失或 ABI 不匹配）：${e.message ?: "无更多信息"}",
        )
    } catch (e: Throwable) {
        SelfTestResult.Unavailable("${e.javaClass.simpleName}: ${e.message}")
    }
}
