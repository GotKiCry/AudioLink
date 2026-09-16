//! 外壳设置（M5）：目前只有「启动时自动连接上次设备」。
//!
//! 为什么用 `tauri-plugin-store` 而不是自己写文件：插件早就注册了却一直没用；
//! 而「原子写 + 读坏了怎么办」这类事它已经处理过，没必要重造一遍。

use serde::{Deserialize, Serialize};

/// 自动重连的决策输入（开关 + 上次连过的地址）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoConnectPolicy {
    /// 用户是否打开这个开关（界面上的勾）。
    pub enabled: bool,
    /// 上一次**成功连接**的地址（`ip:port`）；从没成功过就是 `None`。
    pub last_peer: Option<String>,
}

impl AutoConnectPolicy {
    /// 现在该连哪里；`None` = 什么都不做。
    ///
    /// 只有**开关打开且确实有上次设备**时才重连。刻意不做「扫描一遍、连第一个看到的设备」
    /// 那类事 —— 那等于在用户没要求的情况下连到别人的机器上。
    #[must_use]
    pub fn target(&self) -> Option<&str> {
        if !self.enabled {
            return None;
        }
        self.last_peer.as_deref().filter(|addr| !addr.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(enabled: bool, last_peer: Option<&str>) -> AutoConnectPolicy {
        AutoConnectPolicy {
            enabled,
            last_peer: last_peer.map(str::to_string),
        }
    }

    #[test]
    fn connects_only_when_enabled_and_remembered() {
        assert_eq!(
            policy(true, Some("192.0.2.10:58290")).target(),
            Some("192.0.2.10:58290")
        );
    }

    #[test]
    fn disabled_switch_means_do_nothing() {
        assert_eq!(policy(false, Some("192.0.2.10:58290")).target(), None);
    }

    #[test]
    fn no_history_means_do_nothing() {
        assert_eq!(policy(true, None).target(), None);
    }

    #[test]
    fn empty_history_is_treated_as_no_history() {
        // 存里被写坏成空串时不要拿它去连 —— 那会变成「连本机任意端口」这种怪行为。
        assert_eq!(policy(true, Some("")).target(), None);
    }
}
