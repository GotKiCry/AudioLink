//! audiolink-proto —— ALP/2 帧 / 命令 / 发现报文的**严格编解码层（L1）**
//!
//! 规格：`docs/03-protocol.md`（契约）、`docs/02-architecture.md` §2
//!
//! # 分层约定（`docs/03-protocol.md` §1.1）
//!
//! 本 crate 是 **L1 严格解码层**：任何非法字节流返回 [`AudioLinkError::BadRequest`]（`1008`），**绝不 panic**
//! （`clippy::indexing_slicing` 亦为 deny，所有字节访问都经 `get` / 模式匹配）。
//!
//! 「未知类型忽略并计数、不断流」是 **L2 分发层**（`audiolink-engine`）的职责，本 crate 不实现 ——
//! 严格是为了让 Android / 桌面两端的内核漂移在 CI 立刻暴露（golden vectors 见 `tests/golden_vectors.rs`）。
//!
//! # 模块
//!
//! - [`datagram`]：音频数据报（24 B 帧头 + ptype 专用定长载荷，§3）
//! - [`control`]：控制帧（8 B 帧头 + `request_id` + postcard 载荷，§4）
//! - [`discovery`]：发现报文（UDP 广播二进制 + JSON，mDNS TXT 字段，§9）
//!
//! 三者都**只做编解码**，不做任何 I/O（I/O 属 `audiolink-net` / `audiolink-discovery`）。

#![deny(unsafe_code)] // 必须使用 unsafe 的 crate（如 FFI 绑定）在文件顶部显式 #[allow] 并注明理由
#![deny(clippy::unwrap_used, clippy::expect_used)] // 实时路径禁止 panic；确需处用 #[allow] 并注明理由
#![deny(clippy::panic)]
#![deny(clippy::indexing_slicing)] // L1 契约：越界只能返回 Err，永远不能是 panic

use audiolink_types::AudioLinkError;

pub mod control;
pub mod datagram;
pub mod discovery;

pub use audiolink_types as types;
pub use control::{ControlFrame, ControlFrameHeader, payload_decode, payload_encode};
pub use datagram::{AudioDatagram, AudioDatagramHeader, ClockProbe, ClockReply, NackList};
pub use discovery::{DiscoveryBeacon, DiscoveryTxt};

/// 输出缓冲过小时的统一失败上下文。
pub(crate) const ERR_OUT_OF_SPACE: &str = "output buffer too small";

/// 从 `buf` 的 `at` 偏移处取 `N` 字节定长数组（越界 → `BadRequest`，不 panic）。
pub(crate) fn fixed_bytes<const N: usize>(
    buf: &[u8],
    at: usize,
    what: &'static str,
) -> Result<[u8; N], AudioLinkError> {
    let end = at
        .checked_add(N)
        .ok_or(AudioLinkError::bad_request("offset overflow"))?;
    let slice = buf.get(at..end).ok_or(AudioLinkError::bad_request(what))?;
    <[u8; N]>::try_from(slice).map_err(|_| AudioLinkError::bad_request(what))
}

/// 把 `src` 写入 `dst` 的 `at..at + src.len()` 区间（越界 → `BadRequest`，不 panic）。
pub(crate) fn put(
    dst: &mut [u8],
    at: usize,
    src: &[u8],
    what: &'static str,
) -> Result<(), AudioLinkError> {
    let end = at
        .checked_add(src.len())
        .ok_or(AudioLinkError::bad_request("offset overflow"))?;
    let target = dst
        .get_mut(at..end)
        .ok_or(AudioLinkError::bad_request(what))?;
    target.copy_from_slice(src);
    Ok(())
}
