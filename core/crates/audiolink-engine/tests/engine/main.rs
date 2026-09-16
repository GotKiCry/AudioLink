//! 内核的集成测试：**一个 harness，而不是每个文件一个二进制**。
//!
//! 为什么合并：`tests/*.rs` 的每个**顶层文件**都会被 Cargo 当成独立测试二进制 —— 各自编译一次，
//! 各链接一次整棵 engine + QUIC 栈。实测（本地、依赖已编译）：15 个二进制全部重建要 **7.4 s**，
//! 平均每个约 0.5 s；CI 上这笔开销落在 `core-heavy` 的 test 步骤里。合并后只剩一次链接。
//!
//! 各测试文件**内容一字未改**，只是多了一层 `mod`。单跑某一个仍然可以：
//!
//! ```text
//! cargo nextest run -p audiolink-engine --test engine -E "test(group_epoch)"
//! ```
//!
//! 文件里的 `//!` 与 `#![allow(...)]` 在模块内都是合法的（模块允许内部属性）。

mod adaptive_bitrate;
mod capability_negotiation;
mod clock_sync;
mod dynamic_join;
mod engine_shutdown;
mod gain_control;
mod group_epoch;
mod group_join;
mod group_management;
mod group_sync;
mod handshake_deadline;
mod latency_budget;
mod mixer_alignment;
mod mixer_convergence;
mod multi_session;
mod nack_retransmit;
mod network_outage;
mod pcm_delivery;
mod playout_recovery;
mod receiver_epoch;
