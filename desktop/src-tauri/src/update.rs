//! M5 · 自动更新：`tauri-plugin-updater` 的接线层（检查 → 下载 → 验签 → 安装）。
//!
//! **为什么命令在这一层，而不是让前端直接调插件的 JS API**（`@tauri-apps/plugin-updater`）：
//!
//! 1. **错误必须说人话**（`docs/08-ui-spec.md` §4）。插件自己的 `Error` 是英文机器话
//!    （"Could not fetch a valid release JSON from the remote"），直接扔进界面等于「未知错误」；
//!    要在前端翻译就得在 TS 里按字符串拼原因 —— 那正是本仓禁止的做法。所以翻译只在本模块
//!    一处发生，前端拿到的永远是已经成句的中文（`error::CommandError` 的形状）。
//! 2. **权限面更小**：webview 侧因此拿不到 `plugin:updater|*`
//!    （`capabilities/default.json` 一条 updater 权限都没给），
//!    「静默下载安装」这件事在权限层就没有入口（理由写在该文件里）。
//!
//! **默认行为的边界（重要：不要在这里加"贴心"的自动更新）**：AudioLink 是个音频工具，
//! 用户很可能正在推流或录音。所以本模块**没有**定时器、不在启动时检查、不自动下载、不自动安装：
//!
//! * 检查只能由用户点「检查更新」触发（`check_update`）；
//! * 安装要用户在看到版本号之后**再确认一次** —— `install_update` 要求带上他看到的那个版本号。
//!
//! 执行顺序也是刻意排的：**先下载 + 验签，全部成功之后才动引擎**。
//! `tauri_plugin_updater::Update::download` 内部会用 `tauri.conf.json` 的公钥验签
//! （minisign；篡改必被拒，见 `tests/updater_signature.rs`）。
//! 若反过来先停引擎再下载，一次网络抖动就会白白断掉用户正在进行的会话。

use std::sync::atomic::{AtomicBool, Ordering};

use audiolink_types::ErrorCode;
use serde::Serialize;
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::{Error as UpdaterError, UpdaterExt};

use crate::engine_bridge::EngineBridge;
use crate::error::CommandError;

/// 同一时刻只允许一条更新链路（检查与安装共用一道闸）。
///
/// 为什么要挡：安装路径在 Windows 上会拉起安装器并 `exit(0)`，
/// 用户连点两下按钮就可能同时拉起两个安装器。
static IN_FLIGHT: AtomicBool = AtomicBool::new(false);

/// `IN_FLIGHT` 的 RAII 卫兵：命令返回（成功、失败、被取消）都会把闸放开。
struct Gate;

impl Gate {
    fn acquire(stage: &'static str) -> Result<Self, CommandError> {
        IN_FLIGHT
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| {
                CommandError::busy(
                    "上一次检查或安装还在进行中，请稍候",
                    format!("{stage}: already in flight"),
                )
            })?;
        Ok(Self)
    }
}

impl Drop for Gate {
    fn drop(&mut self) {
        IN_FLIGHT.store(false, Ordering::SeqCst);
    }
}

/// `check_update` 的返回。
///
/// 只有两种**正常**结果：「已是最新」（`version == None`）与「有新版本」（带版本号）。
/// 第三种「出错」不走这里 —— 它是 rejection（`CommandError`），与其它命令同一条路，
/// 前端由 `toCommandError` 统一处理。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckView {
    /// 当前运行版本（= `version` 命令的同一来源：Cargo workspace 版本）。
    pub current_version: String,
    /// 新版本号；`None` = 已是最新。
    pub version: Option<String>,
    /// 新版本的发布说明（`latest.json` 的 `notes`），可能是多行 Markdown 原文。
    pub notes: Option<String>,
    /// 新版本发布日期（`YYYY-MM-DD`，取自 `latest.json` 的 `pub_date`）。
    pub pub_date: Option<String>,
}

/// 当前版本号：与 `version` 命令同源（Cargo workspace 版本）。
fn current_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// 检查更新。**只由用户在界面上主动触发**（理由见模块注释）。
pub async fn check(app: &AppHandle) -> Result<UpdateCheckView, CommandError> {
    let _gate = Gate::acquire("check_update")?;

    // `updater()` 读的是插件在 setup 里登记的 state：插件没装配会直接 panic（不是静默降级）——
    // 这也是「插件到底注册上没有」的硬证据，见 `lib.rs` 的 `.plugin(...)`。
    let update = app
        .updater()
        .map_err(|error| failure("check_update", &error))?
        .check()
        .await
        .map_err(|error| failure("check_update", &error))?;

    Ok(match update {
        None => UpdateCheckView {
            current_version: current_version(),
            version: None,
            notes: None,
            pub_date: None,
        },
        Some(update) => UpdateCheckView {
            current_version: current_version(),
            version: Some(update.version.clone()),
            notes: update.body.clone(),
            // 清单里的时间戳对本机用户没有决策价值，只留日期：`YYYY-MM-DD` 是唯一
            // 不需要按语言格式化的写法（i18n 那边也就不必为它加一个键）。
            pub_date: update.date.map(|date| {
                format!(
                    "{:04}-{:02}-{:02}",
                    date.year(),
                    u8::from(date.month()),
                    date.day()
                )
            }),
        },
    })
}

/// 下载并安装**用户确认过的那个版本**，返回装上的版本号。
///
/// `expected` 必须是用户在上一步界面上看到的版本号：用户同意的是「升级到 X」这件事。
/// 若期间又发了新版本，照旧默默装上就是**超出授权的安装** —— 这里宁可报错，
/// 让他重新检查、重新确认。
pub async fn install(app: &AppHandle, expected: &str) -> Result<String, CommandError> {
    let _gate = Gate::acquire("install_update")?;

    let update = app
        .updater()
        .map_err(|error| failure("install_update", &error))?
        .check()
        .await
        .map_err(|error| failure("install_update", &error))?
        .ok_or_else(|| {
            CommandError::bad_request(
                "这个版本已经不在更新服务器上了，请重新检查更新",
                "install_update: no update available",
            )
        })?;

    if update.version != expected {
        return Err(CommandError::bad_request(
            format!(
                "更新服务器上的最新版本已经变成 {}，请重新检查更新后再确认安装",
                update.version
            ),
            format!(
                "install_update: expected {expected}, got {}",
                update.version
            ),
        ));
    }

    // 下载 + **验签**（公钥来自 `tauri.conf.json` 的 `plugins.updater.pubkey`）。
    // 这一步失败时什么都不做：正在推流的会话不该因为一次下载失败而断掉。
    // 下载进度本轮不往界面推：包只有几 MB，而安装器自己也有进度界面。
    let bytes = update
        .download(|_, _| {}, || {})
        .await
        .map_err(|error| failure("install_update", &error))?;

    // 到这里包已经验签通过、马上要启动安装器 —— 先让引擎体面收尾（给对方发 `BYE`），
    // 与托盘「退出」同一条纪律。因为下一步在 Windows 上会 `exit(0)`，
    // 不走 on_window_event / quit_app 那条路，没有人会替它收尾。
    if let Some(bridge) = app.try_state::<EngineBridge>()
        && let Err(error) = bridge.shutdown_engine().await
    {
        // 收尾失败不阻断升级：用户明确的意图是装上这个新版本。
        tracing::warn!(%error, "更新前收尾未完成，仍然继续安装");
    }

    // Windows（本仓 `bundle.targets = ["nsis"]`）：这一行成功 = 安装器已拉起 + 进程 `exit(0)`，
    // 所以下面的 `Ok` 在 Windows 上是走不到的分支；它给的是其它平台（安装器不接管进程）的返回。
    update
        .install(&bytes)
        .map_err(|error| failure("install_update", &error))?;
    Ok(update.version.clone())
}

/// 插件错误 → `CommandError`（**本仓唯一一处翻译更新失败的地方**）。
fn failure(stage: &str, error: &UpdaterError) -> CommandError {
    // context 放机械原文（排查看这个），message 放人话（用户看那个）—— 与其它命令一致。
    CommandError::new(code_of(error), human(error), format!("{stage}: {error}"))
}

/// 错误码：本仓的错误码是**内核契约**（`ErrorCode` 1001–3001），本轮不扩内核
/// （写范围也只在 desktop/）。于是只按「谁的问题、能不能重试」归类：
///
/// * 环境类（网络不通、连接被打断、启动安装器失败）→ `1009 BUSY`：此刻做不成，等会儿可能成；
/// * 其余（清单缺字段、签名不对、平台不匹配、地址不合规、更新源没配）→ `1008 BAD_REQUEST`：
///   这次拿到的数据/配置本身不合格，重试也没用。
fn code_of(error: &UpdaterError) -> ErrorCode {
    match error {
        UpdaterError::Reqwest(_) | UpdaterError::Network(_) | UpdaterError::Io(_) => {
            ErrorCode::Busy
        }
        _ => ErrorCode::BadRequest,
    }
}

/// 分类依据是「用户下一步能做什么」，不是错误类型的形状：
///
/// * 发布方/清单的问题 → 明说这是发布方的问题，给一条可行的退路（去官网下载）；
/// * 网络问题 → 让他检查网络后重试；
/// * **验签失败** → 明确说「已拒绝安装」并让他重新下载 —— 这是安全承诺，
///   不能被糊成「未知错误」。
fn human(error: &UpdaterError) -> String {
    match error {
        UpdaterError::EmptyEndpoints => {
            "这个版本没有配置更新源，无法自动更新；请到官网下载新版本。".to_string()
        }
        UpdaterError::ReleaseNotFound => {
            "更新服务器没有返回可用的发布清单（latest.json）。可能是这次发布还没上传完，稍后再试。"
                .to_string()
        }
        UpdaterError::TargetNotFound(target) => format!(
            "发布清单里没有适合本机的更新包（{target}）。这通常是发布方的问题，请到官网下载新版本。"
        ),
        UpdaterError::TargetsNotFound(targets) => format!(
            "发布清单里没有适合本机的更新包（{}）。这通常是发布方的问题，请到官网下载新版本。",
            targets.join(", ")
        ),
        UpdaterError::Serialization(error) => {
            format!("更新清单（latest.json）解析失败：{error}。这通常是发布方的问题，请稍后再试。")
        }
        UpdaterError::Minisign(error) => {
            format!("更新包的签名校验没通过（{error}），已拒绝安装。请到官网重新下载安装包。")
        }
        UpdaterError::SignatureUtf8(_) | UpdaterError::Base64(_) => {
            "更新包的签名格式不对，已拒绝安装。请到官网重新下载安装包。".to_string()
        }
        UpdaterError::Network(reason) => {
            format!("下载更新包失败：{reason}。检查网络后重试，或到官网手动下载。")
        }
        UpdaterError::Reqwest(_) => {
            "连不上更新服务器（网络不通或被拦）。检查网络后重试，或到官网手动下载。".to_string()
        }
        UpdaterError::InsecureTransportProtocol => {
            "更新地址不是 https，已按安全策略拒绝这次更新。".to_string()
        }
        UpdaterError::UnsupportedArch | UpdaterError::UnsupportedOs => {
            "这个平台上没有可用的更新包，请到官网下载。".to_string()
        }
        _ => "检查更新失败（原因见日志）。稍后再试，或到官网手动下载。".to_string(),
    }
}

#[cfg(test)]
// 测试里断言失败就该炸：显式放行 panic 系列 lint（工作区默认 deny，见根 Cargo.toml `[workspace.lints]`）。
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    /// 契约字段名一旦漂移，前端会静默拿到 `undefined`；这里用序列化结果钉死形状。
    #[test]
    fn update_check_json_shape_matches_frontend() {
        let up_to_date = UpdateCheckView {
            current_version: "0.1.0".into(),
            version: None,
            notes: None,
            pub_date: None,
        };
        assert_eq!(
            serde_json::to_string(&up_to_date).expect("serialize"),
            r#"{"currentVersion":"0.1.0","version":null,"notes":null,"pubDate":null}"#
        );

        let available = UpdateCheckView {
            current_version: "0.1.0".into(),
            version: Some("0.1.1".into()),
            notes: Some("修了配对超时".into()),
            pub_date: Some("2026-09-17".into()),
        };
        assert_eq!(
            serde_json::to_string(&available).expect("serialize"),
            r#"{"currentVersion":"0.1.0","version":"0.1.1","notes":"修了配对超时","pubDate":"2026-09-17"}"#
        );
    }

    /// 失败必须给出**人话**（用户看 message），机械原文进 context（排查用）。
    #[test]
    fn failures_are_translated_to_human_reasons() {
        let cases: Vec<(UpdaterError, &str)> = vec![
            (UpdaterError::EmptyEndpoints, "更新源"),
            (UpdaterError::ReleaseNotFound, "发布清单"),
            (
                UpdaterError::TargetNotFound("windows-x86_64".into()),
                "windows-x86_64",
            ),
            (UpdaterError::Network("HTTP 503".into()), "下载更新包失败"),
            (
                UpdaterError::SignatureUtf8("not base64".into()),
                "已拒绝安装",
            ),
            (UpdaterError::UnsupportedArch, "没有可用的更新包"),
            (UpdaterError::InsecureTransportProtocol, "https"),
            (
                UpdaterError::Serialization(serde_json::Error::io(std::io::Error::other("eof"))),
                "解析失败",
            ),
        ];

        for (error, expected_fragment) in cases {
            let failure = failure("check_update", &error);
            assert!(
                failure.message.contains(expected_fragment),
                "这句人话里没有 {}：{}",
                expected_fragment,
                failure.message
            );
            assert!(
                failure.context.contains("check_update"),
                "context 该留下阶段与机械原文：{}",
                failure.context
            );
            // 人话里不该出现插件的英文原句（那是 context 的活儿）。
            assert!(!failure.message.contains("Could not fetch"));
        }
    }

    /// 环境类失败可以重试（BUSY），数据类不可以（BAD_REQUEST）——
    /// 前端拿这两个码区分「可以重试」与「重试也没用」。
    #[test]
    fn error_codes_split_retryable_from_data_problems() {
        assert_eq!(code_of(&UpdaterError::Network("x".into())), ErrorCode::Busy);
        assert_eq!(
            code_of(&UpdaterError::EmptyEndpoints),
            ErrorCode::BadRequest
        );
        assert_eq!(
            code_of(&UpdaterError::SignatureUtf8("x".into())),
            ErrorCode::BadRequest
        );
    }
}
