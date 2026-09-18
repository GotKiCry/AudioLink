//! 外壳设置（M5）：自动重连策略 + 应用背景（z0 壁纸）。
//!
//! 为什么用 tauri-plugin-store 而不是自己写文件：插件早就注册了却一直没用；
//! 而「原子写 + 读坏了怎么办」这类事它已经处理过，没必要重造一遍。
//!
//! 存储位置与键空间和 engine_bridge.rs 的 locale / auto_connect 是**同一个**
//! settings.json（本模块与那边各有一份同名常量，改文件名要一起改）—— 用户的选择
//! 必须跨重启活着，而「背景」和「语言」在这一点上没有任何区别。

use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

use crate::error::CommandError;

/// 自动重连的决策输入（开关 + 上次连过的地址）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoConnectPolicy {
    /// 用户是否打开这个开关（界面上的勾）。
    pub enabled: bool,
    /// 上一次**成功连接**的地址（ip:port）；从没成功过就是 None。
    pub last_peer: Option<String>,
}

impl AutoConnectPolicy {
    /// 现在该连哪里；None = 什么都不做。
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

// ---------------------------------------------------------------------------
// 应用背景（z0）：单色 / 双色 / 三色 + 方向
// ---------------------------------------------------------------------------

/// 设置文件名（与 engine_bridge.rs 的同名常量指向同一个文件）。
const SETTINGS_FILE: &str = "settings.json";
/// 背景配置的键。
const KEY_BACKGROUND: &str = "background";

/// 背景模式：单色（纯色）/ 双色 / 三色（线性渐变）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackgroundMode {
    Mono,
    Duo,
    Tri,
}

/// 渐变方向：对角 / 水平 / 垂直。单色模式下无意义（前端把整组禁用置灰）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackgroundDir {
    Diag,
    H,
    V,
}

/// 用户可配的应用背景。
///
/// 与前端 desktop/src/types.ts 的 BackgroundConfig 一字对齐（camelCase；c3 允许空串
/// —— 空串 = 第三个颜色留空，三色按双色处理）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundConfig {
    /// 单色 / 双色 / 三色。
    pub mode: BackgroundMode,
    /// 第一个颜色（单色模式下就是唯一的颜色）。
    pub c1: String,
    /// 第二个颜色。
    pub c2: String,
    /// 第三个颜色；空串 = 留空。
    pub c3: String,
    /// 渐变方向。
    pub dir: BackgroundDir,
}

impl BackgroundConfig {
    /// 深色主题的默认背景（对角三色）。
    ///
    /// 前端（desktop/src/lib/background.ts 的 DEFAULT_BACKGROUND）按生效主题取默认值 ——
    /// 界面上"用户没配过"是 null，不需要后端插一脚。这两个构造函数保留在产品代码里是为了
    /// 「主题默认值必须与 CSS 令牌逐字一致」这件事有可执行的表述与测试（见下方用例）。
    #[allow(dead_code)]
    ///
    /// 与 desktop/src/index.css 里 --al-wall-base / --al-wall-image 的深色默认值
    /// **必须逐字一致**：CSS 那份是「用户没配过」时的底色，这份是浮层的初值与
    /// 「恢复默认」的目标。
    #[must_use]
    pub fn default_dark() -> Self {
        Self {
            mode: BackgroundMode::Tri,
            c1: "#1b2a4a".to_string(),
            c2: "#3d2a6b".to_string(),
            c3: "#0f6b6b".to_string(),
            dir: BackgroundDir::Diag,
        }
    }

    /// 浅色主题的默认背景（对角双色）。一致性约定见 default_dark。
    #[must_use]
    #[allow(dead_code)]
    pub fn default_light() -> Self {
        Self {
            mode: BackgroundMode::Duo,
            c1: "#f4f0e6".to_string(),
            c2: "#a9c8f2".to_string(),
            c3: String::new(),
            dir: BackgroundDir::Diag,
        }
    }

    /// 校验：settings.json 是可以被手改的，写进来一个 not-a-color 不该让界面把背景
    /// 画成透明（那会看着像「软件坏了」）。非法值一律拒掉，前端回落到主题默认。
    pub fn validate(&self) -> Result<(), String> {
        for (label, value) in [("c1", &self.c1), ("c2", &self.c2)] {
            if !is_hex_color(value) {
                return Err(format!("背景色 {label} 不是 #rrggbb 形式：{value}"));
            }
        }
        if !self.c3.is_empty() && !is_hex_color(&self.c3) {
            return Err(format!("背景色 c3 不是 #rrggbb 形式：{}", self.c3));
        }
        Ok(())
    }
}

/// #rrggbb（六位十六进制）。刻意不接受 #rgb / 带 alpha：界面上的取色器给的就是六位，
/// 多一种可接受的写法只是多一种能被写坏的方式。
fn is_hex_color(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/// 读背景配置；None = 用户还没配过（前端用主题默认值）。
///
/// 读坏了（手改成非法 JSON）也返回 None 而不是报错：背景只是外观，不该在启动时给用户
/// 弹一条错误横幅 —— 换成默认背景继续跑才是对的处置。
pub fn read_background(app: &AppHandle) -> Result<Option<BackgroundConfig>, CommandError> {
    let Ok(store) = app.store(SETTINGS_FILE) else {
        return Ok(None);
    };
    let Some(value) = store.get(KEY_BACKGROUND) else {
        return Ok(None);
    };
    Ok(serde_json::from_value(value).ok())
}

/// 保存背景配置（先校验，再落盘）。
pub fn write_background(app: &AppHandle, config: &BackgroundConfig) -> Result<(), CommandError> {
    config
        .validate()
        .map_err(|reason| CommandError::busy(reason, "set_background"))?;
    let store = app
        .store(SETTINGS_FILE)
        .map_err(|error| CommandError::busy(format!("打开设置失败：{error}"), "set_background"))?;
    let value = serde_json::to_value(config)
        .map_err(|error| CommandError::busy(format!("背景配置无法序列化：{error}"), "set_background"))?;
    store.set(KEY_BACKGROUND, value);
    store
        .save()
        .map_err(|error| CommandError::busy(format!("保存设置失败：{error}"), "set_background"))?;
    Ok(())
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

    #[test]
    fn theme_defaults_are_valid_and_match_the_css_tokens() {
        let dark = BackgroundConfig::default_dark();
        let light = BackgroundConfig::default_light();
        assert!(dark.validate().is_ok());
        assert!(light.validate().is_ok());
        // 逐字一致性：index.css 的 --al-wall-* 就是这两个值。
        assert_eq!(dark.c1, "#1b2a4a");
        assert_eq!(dark.c2, "#3d2a6b");
        assert_eq!(dark.c3, "#0f6b6b");
        assert_eq!(light.c1, "#f4f0e6");
        assert_eq!(light.c2, "#a9c8f2");
        // 浅色默认是双色：第三个颜色留空（空串不是「缺字段」，是明确的语义）。
        assert!(light.c3.is_empty());
    }

    #[test]
    fn validate_rejects_broken_colors() {
        let mut config = BackgroundConfig::default_dark();
        config.c1 = "not-a-color".to_string();
        assert!(config.validate().is_err());

        let mut short = BackgroundConfig::default_dark();
        short.c2 = "#fff".to_string();
        assert!(short.validate().is_err());

        // 空串只对 c3 合法（三色留空）；c1/c2 空串必须被拒。
        let mut blank = BackgroundConfig::default_dark();
        blank.c1 = String::new();
        assert!(blank.validate().is_err());
    }

    #[test]
    fn json_shape_is_camel_case() {
        // 前端按 { mode, c1, c2, c3, dir } 读；枚举是小写字面量。
        let json = serde_json::to_value(BackgroundConfig::default_dark()).expect("serialize");
        assert_eq!(json["mode"], "tri");
        assert_eq!(json["dir"], "diag");
        assert_eq!(json["c3"], "#0f6b6b");
        let back: BackgroundConfig = serde_json::from_value(json).expect("deserialize");
        assert_eq!(back, BackgroundConfig::default_dark());
    }
}
