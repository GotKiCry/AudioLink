//! 本机可达地址探测：把「监听地址」翻译成「对端能直接填写的门牌号」。
//!
//! # 为什么不去枚举网卡
//!
//! 枚举网卡要么用平台 API（Windows 的 `GetAdaptersAddresses`），要么引第三方 crate
//! （`if-addrs`）；本项目对依赖有依据要求（`docs/04-tech-stack.md`），而 std 没有这个能力。
//! 但需求比「枚举」窄：只需要知道**默认路由的出口地址**，也就是内核发一个外部报文时会选哪张网卡。
//! `UdpSocket::connect()` 对 UDP 恰好只做这件事 —— 路由查表，**不发送任何报文** ——
//! 之后的 `local_addr()` 就是内核选中的那个出口地址。零依赖、零流量，
//! 没有默认路由时返回 `Err`（下面按「这条路由不存在」处理），不 panic。
//!
//! # 为什么探测目标是文档地址段
//!
//! 目标地址只参与路由查表，不需要真的可达。用 RFC 5737（TEST-NET-1/2/3）与 RFC 3849 的保留段，
//! 是为了万一将来有人把 `connect` 换成 `send_to`，「探测」也不会变成对真实主机的骚扰。
//! 探测**多个**目标是为了覆盖按前缀分流的路由表（VPN 旁路、多条默认路由按 metric 选择）：
//! 每个目标独立探测，出口地址不同的都会进候选。
//!
//! # 什么时候会没有结果
//!
//! 以下情况一律返回空列表（调用方据此如实显示「没有可填的地址」，而不是编一个）：
//! 1. 机器没有默认路由（拔网线、只剩回环）；
//! 2. 出口地址本身不可用（回环 / 未指定 / 链路本地 / 组播 / 广播）；
//! 3. 监听在具体地址上而该地址不可用（典型：集成测试绑 `127.0.0.1`）。

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};

/// 探测目标：文档地址段，见模块文档。
///
/// 前三个按 IPv4 的不同前缀分开探：有 VPN 旁路或多条默认路由时，不同前缀可能落在不同网卡上。
const PROBE_TARGETS: [IpAddr; 4] = [
    IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)), // RFC 5737 TEST-NET-1
    IpAddr::V4(Ipv4Addr::new(198, 51, 100, 1)), // RFC 5737 TEST-NET-2
    IpAddr::V4(Ipv4Addr::new(203, 0, 113, 1)), // RFC 5737 TEST-NET-3
    IpAddr::V6(Ipv6Addr::new(0x2001, 0x0db8, 0, 0, 0, 0, 0, 1)), // RFC 3849
];

/// 探测目标的端口。UDP 下它只参与路由查表，取值本身无意义（写 80 只是可读）。
const PROBE_PORT: u16 = 80;

/// 探测本机出口地址：IPv4 与 IPv6 都探，去重后按「IPv4 优先」排序返回。
///
/// **不保证非空**：没有默认路由的机器返回空列表 —— 这正是调用方要如实告诉用户
/// 「暂时没有可填的地址」的场景，而不是塞一个 `0.0.0.0` 糊弄过去。
pub fn local_endpoints() -> Vec<IpAddr> {
    let mut probed = Vec::with_capacity(PROBE_TARGETS.len());
    for target in PROBE_TARGETS {
        if let Some(address) = probe(target) {
            probed.push(address);
        }
    }
    usable(probed)
}

/// 由实际监听地址推出「对端可以直接填写的候选地址」。
///
/// 两种绑定形态的语义完全不同，这是本函数存在的全部理由：
/// * **通配地址**（`0.0.0.0` / `[::]`）：监听全部网卡，地址只能靠探测得出；
/// * **具体地址**（某张网卡的 IP，或测试里的 `127.0.0.1`）：可达的就是它自己 ——
///   此时拿**别的**网卡的地址去拼端口，等于给用户一个根本连不上的门牌号。
///
/// 具体地址若不通过可达性过滤（典型：绑回环的集成测试），返回空列表：
/// 宁可让界面说「没有可用的局域网地址」，也不要谎报。
pub fn reachable_endpoints(listen: SocketAddr) -> Vec<SocketAddr> {
    let port = listen.port();
    let addresses = if listen.ip().is_unspecified() {
        local_endpoints()
    } else {
        usable([listen.ip()])
    };
    addresses
        .into_iter()
        .map(|address| SocketAddr::new(address, port))
        .collect()
}

/// 单次探测：让内核为 `target` 选一次路由，返回它选中的本机出口地址。
///
/// 返回 `None` 表示「没有到达该目标的路由」，是**预期路径**而非错误：
/// 双栈机器上 IPv6 探测失败、IPv4 成功，就是最常见的一种。
fn probe(target: IpAddr) -> Option<IpAddr> {
    // 地址族必须匹配：IPv4 套接字连不了 IPv6 目标（内核直接返回 Err），反之亦然。
    let bind = match target {
        IpAddr::V4(_) => SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)),
        IpAddr::V6(_) => SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0)),
    };
    let socket = UdpSocket::bind(bind).ok()?;
    // UDP 的 connect 不发包：它只让内核此刻把「去这个目标走哪张网卡」定下来。
    socket.connect(SocketAddr::new(target, PROBE_PORT)).ok()?;
    // 走到这里说明路由存在，此刻的 local_addr 就是出口地址。
    Some(socket.local_addr().ok()?.ip())
}

/// 过滤 → 排序 → 去重：探测结果与「具体绑定」共用的唯一判据。
///
/// **纯函数**：所有关于「什么样的地址才算门牌号」的判断都收在这里，因此可以脱离网络环境单测
/// （没有默认路由的机器上跑测试也必须通过）。排序主键是「IPv4 在前，同族按地址字节序」——
/// 顺序稳定，`displayAddr = lanAddrs[0]` 这条契约在多网卡机器上才可复现。
fn usable(probed: impl IntoIterator<Item = IpAddr>) -> Vec<IpAddr> {
    let mut addresses: Vec<IpAddr> = probed
        .into_iter()
        .map(normalize)
        .filter(|ip| is_usable(*ip))
        .collect();
    addresses.sort_unstable_by_key(sort_key);
    addresses.dedup();
    addresses
}

/// 归一化：`::ffff:a.b.c.d` 只是 IPv4 地址的过渡形态，统一折成 [`IpAddr::V4`]。
///
/// 不折的话有两个实际后果：它会按 IPv6 排到后面（跟「IPv4 优先」的契约相反），
/// 用户还得在手机上多敲一串冒号才能填进去。
fn normalize(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => address,
        },
        IpAddr::V4(_) => address,
    }
}

/// 排序主键：`(地址族, 数值)`。IPv4 记 0 → 排在 IPv6（记 1）前面。
fn sort_key(address: &IpAddr) -> (u8, u128) {
    match address {
        IpAddr::V4(v4) => (0, u128::from(u32::from(*v4))),
        IpAddr::V6(v6) => (1, u128::from(*v6)),
    }
}

/// 这个地址能不能当门牌号给用户填？
///
/// 排除的都是「填了也连不上」或「填了会连到别处」的地址：回环（仅本机）、未指定
/// （`0.0.0.0` 就是本次要修的那个 bug）、链路本地（169.254.x / fe80:: 需要 scope id，
/// 对端猜不出来）、组播与广播（本就不是单播地址）。
///
/// **私网段（10/8、172.16/12、192.168/16）必须保留** —— 那正是局域网本身。
fn is_usable(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => {
            !(v4.is_unspecified()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_multicast()
                || v4.is_broadcast()
                // 0.0.0.0/8「本网络」：is_unspecified 只覆盖 0.0.0.0 这一个值。
                || v4.octets()[0] == 0)
        }
        // IPv4-mapped（::ffff:a.b.c.d）按 IPv4 语义判定：只开 IPv4 的栈会把 IPv6 探测的出口
        // 报成这个形状，直接丢掉就是白白少一个候选。（[`usable`] 已先把它们折成 `IpAddr::V4`，
        // 这里保留同样的判定，好让单独调用本函数的单测也自洽。）
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => is_usable(IpAddr::V4(v4)),
            None => {
                !(v6.is_unspecified()
                    || v6.is_loopback()
                    || v6.is_multicast()
                    // fe80::/10 链路本地：对端得补 scope id 才能用，不是一个可填的地址。
                    || (v6.segments()[0] & 0xffc0) == 0xfe80)
            }
        },
    }
}

#[cfg(test)]
// 测试里断言失败就该炸：显式放行 panic 系列 lint（工作区默认 deny，见根 Cargo.toml `[workspace.lints]`）。
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().expect("测试字面量必须是合法 IP")
    }

    /// 过滤判据必须逐类命中：排除项一个不漏，局域网地址一个不误杀。
    #[test]
    fn unusable_addresses_are_filtered_out() {
        let kept = [
            "192.168.1.23",
            "10.0.0.7",
            "172.16.9.4",
            "100.64.0.1",      // CGNAT，运营商内网也可达
            "240e:3b7:1:2::5", // 全局 IPv6
            "fd00::1",         // ULA，家庭 IPv6 常见形态
        ];
        let dropped = [
            "0.0.0.0",
            "127.0.0.1",
            "169.254.10.1",
            "224.0.0.1",
            "255.255.255.255",
            "0.3.4.5",
            "::",
            "::1",
            "fe80::1",
            "ff02::1",
            "::ffff:127.0.0.1",
        ];
        assert_eq!(usable(kept.iter().map(|t| ip(t))).len(), kept.len());
        assert!(usable(dropped.iter().map(|t| ip(t))).is_empty());
    }

    /// 排序：IPv4 全部排在 IPv6 前面，同族按数值升序，重复项去重。
    #[test]
    fn ordering_is_ipv4_first_then_numeric() {
        let sorted = usable(vec![
            ip("240e::5"),
            ip("192.168.1.23"),
            ip("10.0.0.7"),
            ip("192.168.1.23"),
            ip("10.0.0.7"),
            ip("fd00::1"),
        ]);
        assert_eq!(
            sorted,
            vec![
                ip("10.0.0.7"),
                ip("192.168.1.23"),
                ip("240e::5"),
                ip("fd00::1")
            ]
        );
    }

    /// IPv4-mapped 形态按 IPv4 语义走：折成 `IpAddr::V4` 参与排序，回环的 mapped 形态照样丢掉。
    #[test]
    fn ipv4_mapped_keeps_ipv4_semantics() {
        assert_eq!(
            usable(vec![ip("::ffff:192.168.1.23")]),
            vec![ip("192.168.1.23")]
        );
        assert!(usable(vec![ip("::ffff:127.0.0.1")]).is_empty());
        // 归一化必须发生在排序之前：否则 mapped 地址会被当成 IPv6 排到后面。
        assert_eq!(
            usable(vec![ip("fd00::1"), ip("::ffff:192.168.1.23")]),
            vec![ip("192.168.1.23"), ip("fd00::1")]
        );
    }

    /// 具体监听：只能是它自己（端口原样带上），不能借用别的网卡的地址。
    #[test]
    fn specific_listen_never_borrows_another_nic() {
        let lan = SocketAddr::new(ip("192.168.1.23"), 51234);
        assert_eq!(reachable_endpoints(lan), vec![lan]);

        // 绑回环（集成测试的常态）：如实返回空列表，让界面说「没有可用地址」。
        assert!(reachable_endpoints(SocketAddr::new(ip("127.0.0.1"), 58290)).is_empty());
        assert!(reachable_endpoints(SocketAddr::new(ip("::1"), 58290)).is_empty());
    }

    /// 通配监听：端口取监听端口（不是硬编码），地址全部通过可达性判据。
    #[test]
    fn wildcard_listen_uses_listening_port() {
        for listen in ["0.0.0.0:51234", "[::]:51234"] {
            let listen: SocketAddr = listen.parse().expect("测试字面量");
            let endpoints = reachable_endpoints(listen);
            assert_eq!(
                endpoints,
                reachable_endpoints(listen),
                "同一环境两次调用应一致"
            );
            for endpoint in &endpoints {
                assert_eq!(endpoint.port(), listen.port(), "端口必须来自监听地址");
                assert!(is_usable(endpoint.ip()));
            }
        }
    }

    /// 真实探测：环境相关，因此只断言**不 panic、结果自洽、可重复**。
    /// 不断言非空 —— 拔网线的机器与 CI 沙箱里，空列表才是正确答案。
    #[test]
    fn probing_is_safe_without_a_default_route() {
        let first = local_endpoints();
        assert_eq!(first, local_endpoints(), "同一环境两次探测结果应一致");
        for address in &first {
            assert!(is_usable(*address), "{address} 不该出现在候选里");
        }
        if let Some(index) = first.iter().position(IpAddr::is_ipv6) {
            assert!(
                first[index..].iter().all(IpAddr::is_ipv6),
                "IPv4 必须排在前面：displayAddr 取的就是第一条"
            );
        }
    }
}
