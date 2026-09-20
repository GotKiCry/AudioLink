package com.gotkicry.audiolink.ui.host

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * [LocalAddresses] 的口径测试（JVM，无 Android 依赖）。
 *
 * 为什么这些断言值得逐条钉死：这张卡上的字符串是**要被用户抄到另一台设备里去的**。
 * 一旦过滤松一格（例如漏了 0.0.0.0 或 127.0.0.1），用户看到的就不是报错，而是一个
 * 「怎么输都连不上」的地址 —— 他会先去怀疑 Wi-Fi、手机、防火墙，最后才怀疑界面。
 * 所以这里的判据与内核 `audiolink-net::local_addr` 的 `is_usable` 逐条对齐，两侧各测一份。
 *
 * 探测（`DatagramSocket.connect`）本身不进测试：它在没有默认路由的机器上必然返回空，
 * 拿真实网络环境断言等于把测试挂在网线上。所有用例都注入 probe 实现。
 */
class LocalAddressesTest {

    // ── 绝不能上屏的地址 ────────────────────────────────────────────

    /**
     * 通配与回环：**这类地址在对方设备上指向的是对方自己** —— 恰好是本次要修的那个 bug
     * （旧版把监听地址 `0.0.0.0:58290` 印在诊断面板里）。
     */
    @Test
    fun wildcardAndLoopbackAreNeverPresentable() {
        val rejected = listOf(
            "0.0.0.0",
            "0.0.0.0:58290",
            "[::]:58290",
            "::",
            "::1",
            "127.0.0.1",
            "127.0.0.1:58290",
            "0.3.4.5", // 0.0.0.0/8 整段：isAnyLocalAddress 只覆盖 0.0.0.0 这一个值
        )
        for (raw in rejected) {
            assertEquals("这条地址不该被出示：$raw", "", LocalAddresses.usableAddr(raw))
        }
    }

    /** 空、空白、null、以及**不是 IP 字面量的东西**：一律当「没有地址」，不猜。 */
    @Test
    fun emptyAndNonLiteralInputsYieldNothing() {
        val rejected = listOf(null, "", "   ", "localhost", "localhost:58290", "audiolink.local", "192.168.1", "999.1.1.1")
        for (raw in rejected) {
            assertEquals("这条输入不该被出示：$raw", "", LocalAddresses.usableAddr(raw))
        }
    }

    /**
     * 链路本地 / 组播 / 广播：内核 `is_usable` 同样排除。
     * 169.254 与 fe80:: 需要 scope id，对端猜不出来；组播与广播本就不是单播地址。
     */
    @Test
    fun linkLocalMulticastAndBroadcastAreRejected() {
        val rejected = listOf("169.254.10.1", "224.0.0.1", "255.255.255.255", "[fe80::1]:58290", "fe80::1", "ff02::1")
        for (raw in rejected) {
            assertEquals("这条地址不该被出示：$raw", "", LocalAddresses.usableAddr(raw))
        }
    }

    // ── 局域网地址一个都不能误杀 ────────────────────────────────────

    /**
     * 私网段必须原样保留（**那正是局域网本身**），端口也要原样带出来。
     * 带端口的形态尤其要保住：内核 `parse_addr` 认它，而端口可能是系统分配的随机值。
     */
    @Test
    fun lanAddressesSurviveTheFilter() {
        val kept = listOf(
            "192.168.1.23",
            "10.0.0.7",
            "172.16.9.4",
            "100.64.0.1",        // CGNAT：运营商内网同样可达
            "192.168.1.5:58290",
            "[fd00::1]:58290",
            "240e:3b7:1:2::5",   // 全局 IPv6
            "fd00::1",           // ULA：家庭 IPv6 的常见形态
        )
        for (raw in kept) {
            assertEquals("这条地址必须原样保留：$raw", raw, LocalAddresses.usableAddr(raw))
        }
    }

    /** 前后空白是输入层的常态（粘贴带空格），修掉后照常判可用。 */
    @Test
    fun surroundingWhitespaceIsTrimmed() {
        assertEquals("192.168.1.5:58290", LocalAddresses.usableAddr("  192.168.1.5:58290 "))
        assertEquals("", LocalAddresses.usableAddr("  0.0.0.0:58290 "))
    }

    // ── 端口 ───────────────────────────────────────────────────────

    /** 端口取自内核**实际**监听的地址：硬编码 58290 在系统随机分配时会印错。 */
    @Test
    fun portComesFromTheListenAddressOnly() {
        assertEquals(58290, LocalAddresses.portOf("0.0.0.0:58290"))
        assertEquals(58290, LocalAddresses.portOf("[::]:58290"))
        assertEquals(1_234, LocalAddresses.portOf("192.168.1.5:1234"))
        // 没有端口就不该编一个出来（裸 IP 交给内核走缺省端口，那是内核的事）。
        assertEquals(null, LocalAddresses.portOf("0.0.0.0"))
        assertEquals(null, LocalAddresses.portOf("192.168.1.5"))
        assertEquals(null, LocalAddresses.portOf(""))
        assertEquals(null, LocalAddresses.portOf(null))
        // 越界端口是坏数据，不是「端口 70000」。
        assertEquals(null, LocalAddresses.portOf("0.0.0.0:70000"))
        assertEquals(null, LocalAddresses.portOf("0.0.0.0:0"))
        // 裸 IPv6 的最后一段冒号后面不是端口（`fe80::1` 的 `1` 是地址的一部分）。
        assertEquals(null, LocalAddresses.portOf("fd00::1"))
    }

    /** IPv6 拼端口必须带方括号：否则对方按最后一个冒号切，地址就被切坏了。 */
    @Test
    fun ipv6IsBracketedWhenPortIsAttached() {
        assertEquals("[fd00::1]:58290", LocalAddresses.format("fd00::1", 58290))
        assertEquals("fd00::1", LocalAddresses.format("fd00::1", null))
        assertEquals("192.168.1.5:58290", LocalAddresses.format("192.168.1.5", 58290))
        assertEquals("192.168.1.5", LocalAddresses.format("192.168.1.5", null))
        assertEquals("", LocalAddresses.format("", 58290))
    }

    // ── 探测结果的挑选 ─────────────────────────────────────────────

    /** 显示的就是第一条可用候选；坏候选（0.0.0.0）会被跳过而不是阻塞。 */
    @Test
    fun displayAddrTakesTheFirstUsableCandidate() {
        val candidates = listOf("0.0.0.0", "127.0.0.1", "192.168.1.23", "10.0.0.7")
        assertEquals("192.168.1.23:58290", LocalAddresses.displayAddr(candidates, 58290))
        assertEquals("192.168.1.23", LocalAddresses.displayAddr(candidates, null))
    }

    /** 一条候选都没有 → 空串。界面据此显示「未检测到局域网地址」，而不是显示监听地址。 */
    @Test
    fun displayAddrIsEmptyWhenNothingIsUsable() {
        assertEquals("", LocalAddresses.displayAddr(emptyList(), 58290))
        assertEquals("", LocalAddresses.displayAddr(listOf("0.0.0.0", "127.0.0.1"), 58290))
    }

    /**
     * 探测顺序即优先级：IPv4 全部排在 IPv6 前面（内核同款口径），同一张网卡被多个目标
     * 命中时只留一条（多目标落在同一出口是常态，不是异常）。
     */
    @Test
    fun discoveryFiltersDeduplicatesAndPutsIpv4First() {
        // 兜底来源一次给四条：0.0.0.0 被丢弃、192.168 重复、IPv6 落在最后。
        val addresses = LocalAddresses.lanAddrs(
            activeNetwork = { emptyList() },
            interfaces = { emptyList() },
            routeProbe = { listOf("0.0.0.0", "192.168.1.23", "192.168.1.23", "fd00::1") },
        )

        assertEquals(listOf("192.168.1.23", "fd00::1"), addresses)
    }

    // ---- 三个来源的优先级与合并（真机缺陷的回归）----

    /**
     * 主来源（ConnectivityManager 视角的当前网络）单独就够了 —— **不依赖 UDP 探测**。
     *
     * 这是 2026-09 真机缺陷的回归：PHK110 上 `ip route get 192.0.2.1` 明明说走 wlan0，
     * 而应用里裸 `DatagramSocket` 的出口是空的（per-UID 策略路由）。旧实现只有那一条来源，
     * 界面于是显示「未检测到局域网地址」。现在只要系统给了当前网络，地址就必须出来。
     */
    @Test
    fun theActiveNetworkAloneIsEnough() {
        val addresses = LocalAddresses.lanAddrs(
            activeNetwork = { listOf("192.168.3.75") },
            interfaces = { emptyList() },
            routeProbe = { emptyList() },
        )

        assertEquals(listOf("192.168.3.75"), addresses)
    }

    /** 三来源合并：主来源在前、补充来源补位、重复只留一条、坏地址全部丢掉。 */
    @Test
    fun sourcesAreMergedDeduplicatedAndFiltered() {
        val addresses = LocalAddresses.lanAddrs(
            activeNetwork = { listOf("192.168.3.75", "0.0.0.0") },
            interfaces = { listOf("192.168.3.75", "10.0.0.7", "127.0.0.1") },
            routeProbe = { listOf("fd00::1", "10.0.0.7") },
        )

        assertEquals(listOf("192.168.3.75", "10.0.0.7", "fd00::1"), addresses)
    }

    /** 只有网卡枚举也要能出地址（ConnectivityManager 不可用的场景）。 */
    @Test
    fun interfacesAloneAreEnough() {
        val addresses = LocalAddresses.lanAddrs(
            activeNetwork = { emptyList() },
            interfaces = { listOf("192.168.3.75") },
            routeProbe = { emptyList() },
        )

        assertEquals(listOf("192.168.3.75"), addresses)
    }

    /** 只有 UDP 兜底也要能出地址（老路径仍然有效）。 */
    @Test
    fun routeProbeAloneIsStillEnough() {
        val addresses = LocalAddresses.lanAddrs(
            activeNetwork = { emptyList() },
            interfaces = { emptyList() },
            routeProbe = { listOf("192.168.3.75") },
        )

        assertEquals(listOf("192.168.3.75"), addresses)
    }

    /** 三个来源都给不出东西 → 空列表。这时界面说「未检测到局域网地址」才是实话。 */
    @Test
    fun allSourcesEmptyYieldsNothing() {
        assertEquals(
            emptyList<String>(),
            LocalAddresses.lanAddrs(
                activeNetwork = { emptyList() },
                interfaces = { emptyList() },
                routeProbe = { emptyList() },
            ),
        )
    }

    /**
     * 探测目标本身也要守住这条：IPv4 目标必须排在 IPv6 之前 ——
     * 第一条命中的地址就是显示给用户的那一条，而双栈机器上 IPv6 常常是「探测得到但对方连不上」。
     */
    @Test
    fun probeTargetsAskForIpv4BeforeIpv6() {
        val targets = LocalAddresses.PROBE_TARGETS
        val firstIpv6 = targets.indexOfFirst { it.contains(':') }
        assertTrue("探测目标里必须有 IPv6（双栈机器只有 IPv6 出口时不能空手而归）", firstIpv6 > 0)
        assertTrue(
            "IPv4 目标必须全部排在 IPv6 之前",
            targets.take(firstIpv6).none { it.contains(':') },
        )
    }
}
