package com.gotkicry.audiolink.ui.host

import android.content.Context
import android.net.ConnectivityManager
import android.util.Log
import java.net.DatagramSocket
import java.net.Inet4Address
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.NetworkInterface

/**
 * 本机地址：内核给的 `addr` 是**监听**地址（绑通配地址时恒为 `0.0.0.0:58290`），
 * 而「主机出示给接收端去填」的那个门牌号必须另算 —— 前者抄到对方设备上，指向的是对方自己。
 *
 * 口径与内核 `audiolink-net::local_addr` **刻意保持一致**（同一套可用性判据、同一批探测目标）：
 * 界面与内核各算一份地址，两边的「可用」定义一旦不同，用户就会在界面上看到一个内核根本不认的地址。
 *
 * 三项分工：
 * * [lanAddrs]：探测本机出口地址（零依赖、不发包）；
 * * [usableAddr]：单条地址的可达性过滤 —— 界面是最后一道闸（见其注释）；
 * * [displayAddr] / [portOf] / [format]：拼出「ip:port」这个可抄写的字符串。
 *
 * 探测**三个来源合并**（顺序即优先级，见 [lanAddrs]）：ConnectivityManager 为主、网卡枚举为辅、
 * UDP 路由查表兜底。真机上曾经只靠最后一条，结果是"手机 Wi-Fi 满格、界面却说没有地址"。
 */
object LocalAddresses {

    /** 定位日志的 TAG：`adb logcat -s AudioLink` 就能看到每次探测的来源与结果。 */
    private const val TAG = "AudioLink"

    /** 探测目标端口：UDP 下只参与路由查表，取值本身无意义（写 80 只是可读）。 */
    private const val PROBE_PORT = 80

    /**
     * 探测目标：RFC 5737（TEST-NET-1/2/3）与 RFC 3849 的文档地址段。
     *
     * 三个 IPv4 目标按不同前缀分开探 —— 有 VPN 旁路或多条默认路由时，不同前缀可能落在不同网卡上。
     * **顺序即优先级**（见 [lanAddrs]）：第一条命中的就是默认路由的出口。
     */
    internal val PROBE_TARGETS: List<String> =
        listOf("192.0.2.1", "198.51.100.1", "203.0.113.1", "2001:db8::1")

    /**
     * 本机可供对方填写的候选地址（IPv4 优先、去重、已过 [usableAddr] 过滤）。
     *
     * **三个来源按优先级合并**，顺序即优先级：
     * 1. [activeNetwork]：`ConnectivityManager` 的当前网络 —— **主来源**。它回答的正是
     *    "本应用此刻能用哪张网卡"，在 Android 的 per-UID 策略路由下这是唯一权威的答案；
     * 2. [interfaces]：`NetworkInterface` 枚举（过滤 down/回环）—— 补充来源；
     * 3. [routeProbe]：UDP 路由查表 —— 兜底。
     *
     * **为什么不能只留第 3 条**（2026-09 真机取证）：PHK110 上 `ip route get 192.0.2.1` 明明给出
     * `src 192.168.3.75 dev wlan0`，可应用里裸 `DatagramSocket` 的出口却是空 —— per-UID 策略路由
     * 下，应用能走的表和 shell 看到的不是同一张，异常又被 `catch (Throwable)` 吞掉，界面于是"如实"
     * 显示了「未检测到局域网地址」。**地址是设备属性**：不该依赖引擎是否启动，也不该只在某一种
     * 探测方式恰好管用时才出现。
     *
     * **不保证非空**：三个来源都拿不到才返回空列表 —— 那时候界面说「未检测到」才是实话。
     * 私网段（10/8、172.16/12、192.168/16）一律保留。
     *
     * @param activeNetwork 当前网络（应用视角）的地址；默认空 —— 纯逻辑层不碰 Android API，
     *   真机入口见 [lanAddrs]（Context 重载）。
     */
    fun lanAddrs(
        activeNetwork: () -> List<String> = { emptyList() },
        interfaces: () -> List<String> = ::interfaceAddrs,
        routeProbe: () -> List<String> = ::probeRouteAddrs,
    ): List<String> {
        // 保序去重：三个来源大概率指向同一张网卡（这是常态，不是异常）。
        val found = LinkedHashSet<String>()
        for (raw in activeNetwork() + interfaces() + routeProbe()) {
            val address = usableAddr(raw)
            if (address.isNotEmpty()) found += address
        }
        return found.filterNot { it.contains(':') } + found.filter { it.contains(':') }
    }

    /**
     * 真机入口：三来源合并 + 一条定位日志。
     *
     * 日志只打"每个来源贡献了什么、合并结果是什么"，每次探测一行（首帧或用户点「重新检测」时）。
     * 以后再遇到"没有地址"，`adb logcat -s AudioLink` 一眼就能看出是哪一层没给 ——
     * 上一轮这个缺陷之所以要真机取证才定位，就是因为这里没有任何可查的痕迹。
     */
    fun lanAddrs(context: Context): List<String> {
        // 单个来源失败不该拖垮整体：任何一层抛异常都只让它贡献空列表。
        val active = runCatching { activeNetworkAddrs(context) }.getOrDefault(emptyList())
        val nics = runCatching { interfaceAddrs() }.getOrDefault(emptyList())
        val route = runCatching { probeRouteAddrs() }.getOrDefault(emptyList())
        val merged = lanAddrs({ active }, { nics }, { route })
        runCatching {
            Log.i(TAG, "host address: activeNetwork=$active interfaces=$nics routeProbe=$route -> $merged")
        }
        return merged
    }

    /** `ConnectivityManager` 的当前网络地址 —— 应用**真正可用**的那张网卡。 */
    private fun activeNetworkAddrs(context: Context): List<String> {
        val manager = context.getSystemService(ConnectivityManager::class.java) ?: return emptyList()
        val network = manager.activeNetwork ?: return emptyList()
        val link = manager.getLinkProperties(network) ?: return emptyList()
        return link.linkAddresses.mapNotNull { it.address?.hostAddress }
    }

    /** 网卡枚举：只要 up 且非回环的接口（虚拟接口/别名交给 [usableAddr] 统一过滤）。 */
    private fun interfaceAddrs(): List<String> = try {
        NetworkInterface.getNetworkInterfaces()
            .toList()
            .filter { nic -> nic.isUp && !nic.isLoopback }
            .flatMap { nic -> nic.interfaceAddresses.mapNotNull { it.address?.hostAddress } }
    } catch (_: Throwable) {
        emptyList()
    }

    /** UDP 兜底：对每个探测目标做一次路由查表，收集出口地址。 */
    private fun probeRouteAddrs(): List<String> {
        val found = LinkedHashSet<String>()
        for (target in PROBE_TARGETS) {
            val address = usableAddr(probeRoute(target))
            if (address.isNotEmpty()) found += address
        }
        return found.toList()
    }

    /**
     * 这条地址能不能交给对方设备去填；不能就给空串。
     *
     * 界面是**最后一道闸**：地址由内核/探测给出，而「拿监听地址兜底」是一类很容易复发的退化。
     * 宁可在屏幕上写「未检测到局域网地址」，也不要印一串对方永远连不上的字符 ——
     * 那种情况下用户会先去怀疑 Wi-Fi、手机、防火墙，最后才怀疑界面。
     *
     * 排除项与内核 `is_usable` 逐条对齐：通配（`0.0.0.0` / `::`）、回环（127/8、`::1`）、
     * 链路本地（169.254/16、fe80::/10 —— 对端补不出 scope id）、组播、广播，以及 `0.0.0.0/8` 整段。
     * **私网段（10/8、172.16/12、192.168/16）必须保留** —— 那正是局域网本身。
     *
     * @param raw `192.168.1.5`、`192.168.1.5:58290` 或 `[fe80::1]:58290`；null 与空串同样返回空串。
     * @return 可用时**原样**返回（保留端口部分），否则空串。
     */
    fun usableAddr(raw: String?): String {
        val text = raw?.trim().orEmpty()
        if (text.isEmpty()) return ""
        val address = literalOrNull(hostOf(text)) ?: return ""
        return if (isUsable(address)) text else ""
    }

    /**
     * 要显示的那一条地址（含端口）；候选为空时返回空串。
     *
     * 只取第一条：多网卡时并列显示会把「抄哪个」变成一道选择题，而第一条（默认路由出口）
     * 在绝大多数场景下就是对的，其余候选由「本机有多个地址」提示交代。
     *
     * @param port 内核实际监听的端口；为 null（引擎没起来）时给裸 IP —— 内核 `parse_addr`
     *   的缺省端口与它相同，对方只填 IP 一样连得上。
     */
    fun displayAddr(candidates: List<String>, port: Int?): String {
        val first = candidates.firstOrNull { usableAddr(it).isNotEmpty() } ?: return ""
        return format(first, port)
    }

    /** 从内核的监听地址（`0.0.0.0:58290` / `[::]:58290`）里取端口；取不到返回 null。 */
    fun portOf(listenAddr: String?): Int? {
        val text = listenAddr?.trim().orEmpty()
        val tail = when {
            text.startsWith("[") -> text.substringAfter("]:", "")
            text.count { it == ':' } == 1 -> text.substringAfter(':', "")
            else -> ""
        }
        return tail.toIntOrNull()?.takeIf { it in 1..65_535 }
    }

    /** 拼「ip:port」。IPv6 必须带方括号：否则对方按「最后一个冒号」解析，会把地址切坏。 */
    fun format(host: String, port: Int?): String = when {
        host.isEmpty() -> ""
        host.contains(':') && port != null -> "[${host}]:$port"
        port != null -> "$host:$port"
        else -> host
    }

    /**
     * 单次探测：让内核为 [target] 选一次路由，返回它选中的本机出口地址。
     *
     * 返回 null 表示「没有到达该目标的路由」，是**预期路径**而非错误：双栈机器上
     * IPv6 探测失败、IPv4 成功，就是最常见的一种。
     */
    private fun probeRoute(target: String): String? {
        val remote = literalOrNull(target) ?: return null
        val socket = try {
            DatagramSocket()
        } catch (_: Throwable) {
            // 套接字建不起来（极端资源紧张）：当作「这条路由探不到」。
            // 后果由界面如实呈现（说没有地址），不把异常抛给调用方 —— 首屏不该因为一次探测失败而崩。
            return null
        }
        return try {
            socket.connect(InetSocketAddress(remote, PROBE_PORT))
            socket.localAddress?.hostAddress
        } catch (_: Throwable) {
            null
        } finally {
            runCatching { socket.close() }
        }
    }

    /** 从「地址（可带端口）」里取出主机部分：`[v6]:port` 去括号，`v4:port` 去端口，裸地址原样。 */
    private fun hostOf(text: String): String {
        if (text.startsWith("[")) {
            val end = text.indexOf(']')
            return if (end > 0) text.substring(1, end) else text
        }
        // 只有一个冒号才是「ipv4:port」；裸 IPv6 有多个冒号，原样返回。
        return if (text.count { it == ':' } == 1) text.substringBefore(':') else text
    }

    /**
     * 地址字面量 → [InetAddress]；不是字面量（域名、`localhost`）返回 null。
     *
     * 只认字面量是刻意的：`InetAddress.getByName` 对域名会发起一次 DNS 查询，而本方法会被
     * 界面在每次组合时调用。主机名形态也不需要支持 —— `localhost` 指向本机自己，交给对方填必然连不上。
     */
    private fun literalOrNull(text: String): InetAddress? {
        if (text.isEmpty()) return null
        val looksIpv6 = text.contains(':') &&
            text.all { it.isDigit() || it == ':' || it in "abcdefABCDEF." }
        if (!looksLikeIpv4(text) && !looksIpv6) return null
        return try {
            InetAddress.getByName(text)
        } catch (_: Throwable) {
            // 形态像字面量但解析不了（例如 999.1.1.1）：当作没有地址，不抛。
            null
        }
    }

    /**
     * 严格四段点分十进制 —— **不能把这件事交给 [InetAddress.getByName]**。
     *
     * 它对 IPv4 的解析是历史包袱式的宽松：`192.168.1` 会被当成 `192.168.1.0`、
     * `010.1.1.1` 会被当成八进制（即 `8.1.1.1`）。一张教人抄地址的卡片如果原样放行这些形式，
     * 用户抄到的就是**另一个地址** —— 这正是本卡存在的意义所反对的（宁可说没有地址）。
     */
    private fun looksLikeIpv4(text: String): Boolean {
        val parts = text.split('.')
        if (parts.size != 4) return false
        return parts.all { part ->
            // 空段、超长段、非数字段、前导零（八进制陷阱）一律不接受。
            part.isNotEmpty() &&
                part.length <= 3 &&
                (part.length == 1 || part[0] != '0') &&
                part.all { it.isDigit() } &&
                part.toInt() <= 255
        }
    }

    /** 与内核 `is_usable` 同一套判据（逐条理由见 [usableAddr]）。 */
    private fun isUsable(address: InetAddress): Boolean {
        val bytes = address.address
        // IPv4-mapped（::ffff:a.b.c.d）按 IPv4 语义判定：只开 IPv4 的栈会把 IPv6 探测的出口
        // 报成这个形状，直接丢掉就是白白少一个候选（内核侧同样折叠）。
        if (bytes.size == 16 && isV4Mapped(bytes)) {
            return isUsable(InetAddress.getByAddress(bytes.copyOfRange(12, 16)))
        }
        if (bytes.size == 4 || address is Inet4Address) {
            val first = bytes[0].toInt() and 0xFF
            return !(address.isAnyLocalAddress ||
                address.isLoopbackAddress ||
                address.isLinkLocalAddress ||
                address.isMulticastAddress ||
                // 255.255.255.255 广播：本就不是单播地址。
                bytes.all { (it.toInt() and 0xFF) == 0xFF } ||
                // 0.0.0.0/8「本网络」：isAnyLocalAddress 只覆盖 0.0.0.0 这一个值。
                first == 0)
        }
        return !(address.isAnyLocalAddress ||
            address.isLoopbackAddress ||
            address.isLinkLocalAddress ||
            address.isMulticastAddress)
    }

    /** `::ffff:a.b.c.d` 的前 10 字节为 0、第 11/12 字节为 0xFF。 */
    private fun isV4Mapped(bytes: ByteArray): Boolean =
        bytes.size == 16 &&
            bytes.take(10).all { it.toInt() == 0 } &&
            (bytes[10].toInt() and 0xFF) == 0xFF &&
            (bytes[11].toInt() and 0xFF) == 0xFF
}
