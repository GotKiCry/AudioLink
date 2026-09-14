//! 信任库（白名单）—— `docs/02-architecture.md` §8/§9、`docs/03-protocol.md` §5
//!
//! 桌面端落 `%APPDATA%\AudioLink\trust.json`，Android 落应用私有目录（路径由调用方给，本 crate 不猜）。
//!
//! 两条刻意的严格性（都会在验收里被断言）：
//!
//! 1. **文件不存在 = 空库**（首次运行），不是错误；
//! 2. **文件存在但读不懂 = `Err`**，绝不静默重置 —— 信任库是安全边界，默默清空它等于把
//!    已配对设备变回「未配对」，而用户在 UI 上看到的只是一次正常启动。
//!
//! 白名单只存指纹（[`NodeId`]）+ 展示元数据；**信任判定只看完整指纹**，`name` / `platform`
//! 一律不参与（架构 §3：展示名仅展示）。

use std::fs;
use std::path::{Path, PathBuf};

use audiolink_types::{NodeId, Platform};
use serde::{Deserialize, Serialize};

use crate::atomic::write_atomic;
use crate::error::IdentityError;

/// 信任库文件格式版本（写入时固定；读到别的版本直接拒绝，不做前向兼容猜测）。
const TRUST_FILE_VERSION: u32 = 1;

/// 一条信任记录（`docs/11-m1-contract.md` §4）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustEntry {
    /// 对端指纹（完整 32 B）。
    pub id: NodeId,
    /// 对端展示名（仅展示）。
    pub name: String,
    /// 对端平台（仅展示）。
    pub platform: Platform,
    /// 首次配对成功的 Unix 秒（挂钟；只用于展示「何时配对」）。
    pub paired_at_unix: u64,
}

/// 白名单持久化。
#[derive(Debug)]
pub struct TrustStore {
    path: PathBuf,
    entries: Vec<TrustEntry>,
}

impl TrustStore {
    /// 从 `path` 加载。
    ///
    /// 不存在 → 空库（不是错误）。文件损坏 → `Err`（**不静默重置**，否则信任边界被悄悄清空）。
    pub fn load(path: impl AsRef<Path>) -> Result<Self, IdentityError> {
        let path = path.as_ref().to_path_buf();
        if !path.exists() {
            return Ok(Self {
                path,
                entries: Vec::new(),
            });
        }

        let text = fs::read_to_string(&path).map_err(|error| {
            IdentityError::trust_store_owned(format!("信任库 {} 读取失败：{error}", path.display()))
        })?;
        let file: TrustFile = serde_json::from_str(&text).map_err(|error| {
            IdentityError::trust_store_owned(format!(
                "信任库 {} 不是合法 JSON（拒绝重置为空库，请人工确认）：{error}",
                path.display()
            ))
        })?;

        if file.version != TRUST_FILE_VERSION {
            return Err(IdentityError::trust_store_owned(format!(
                "信任库 {} 的格式版本是 {}，本实现只认 {TRUST_FILE_VERSION}",
                path.display(),
                file.version
            )));
        }

        let mut entries: Vec<TrustEntry> = Vec::with_capacity(file.entries.len());
        for raw in file.entries {
            let id = NodeId::from_hex(&raw.id).ok_or_else(|| {
                IdentityError::trust_store_owned(format!(
                    "信任库 {} 含非法指纹（需要 64 字符 hex）：{:?}",
                    path.display(),
                    raw.id
                ))
            })?;
            if entries.iter().any(|existing| existing.id == id) {
                return Err(IdentityError::trust_store_owned(format!(
                    "信任库 {} 含重复指纹 {}",
                    path.display(),
                    id.to_hex()
                )));
            }
            entries.push(TrustEntry {
                id,
                name: raw.name,
                platform: parse_platform(&raw.platform),
                paired_at_unix: raw.paired_at_unix,
            });
        }

        Ok(Self { path, entries })
    }

    /// 该指纹是否已信任（信任判定的唯一依据）。
    pub fn is_trusted(&self, id: NodeId) -> bool {
        self.entries.iter().any(|entry| entry.id == id)
    }

    /// 写入并**原子落盘**；同一 `NodeId` 重复写入为更新。
    pub fn trust(&mut self, entry: TrustEntry) -> Result<(), IdentityError> {
        match self
            .entries
            .iter_mut()
            .find(|existing| existing.id == entry.id)
        {
            Some(existing) => *existing = entry,
            None => self.entries.push(entry),
        }
        self.save()
    }

    /// 撤销信任（FR-18）；返回是否确实删掉了。
    ///
    /// 没删掉时不写盘：避免为一次空操作制造无谓的磁盘写入窗口。
    pub fn revoke(&mut self, id: NodeId) -> Result<bool, IdentityError> {
        let before = self.entries.len();
        self.entries.retain(|entry| entry.id != id);
        if self.entries.len() == before {
            return Ok(false);
        }
        self.save()?;
        Ok(true)
    }

    /// 全部信任记录（顺序 = 落盘顺序）。
    pub fn entries(&self) -> &[TrustEntry] {
        &self.entries
    }

    /// 序列化并原子落盘。
    fn save(&self) -> Result<(), IdentityError> {
        let file = TrustFile {
            version: TRUST_FILE_VERSION,
            entries: self
                .entries
                .iter()
                .map(|entry| TrustFileEntry {
                    id: entry.id.to_hex(),
                    name: entry.name.clone(),
                    platform: entry.platform.as_str().to_owned(),
                    paired_at_unix: entry.paired_at_unix,
                })
                .collect(),
        };
        let mut json = serde_json::to_string_pretty(&file).map_err(|error| {
            IdentityError::trust_store_owned(format!("信任库序列化失败：{error}"))
        })?;
        json.push('\n');
        write_atomic(&self.path, json.as_bytes()).map_err(|error| {
            IdentityError::trust_store_owned(format!(
                "信任库 {} 落盘失败：{}",
                self.path.display(),
                error.context()
            ))
        })
    }
}

/// 落盘的 JSON 结构（对外格式，改动即破坏兼容）。
#[derive(Debug, Serialize, Deserialize)]
struct TrustFile {
    version: u32,
    entries: Vec<TrustFileEntry>,
}

/// 单条落盘记录：指纹用 64 字符 hex（人可读、可人工审计白名单）。
#[derive(Debug, Serialize, Deserialize)]
struct TrustFileEntry {
    id: String,
    name: String,
    platform: String,
    paired_at_unix: u64,
}

/// 解析平台文本。
///
/// 未知取值一律落 [`Platform::Unknown`] 而**不丢弃整条记录**：平台只是展示元数据，
/// 不参与信任判定；为了一个展示字段丢掉一条信任记录，等于悄悄缩小白名单。
/// （对比：`docs/03-protocol.md` §9.2 的发现报文可以丢弃未知平台的报文 —— 那里丢的是
/// 「一次发现提示」，这里丢的是「信任关系」。）
fn parse_platform(text: &str) -> Platform {
    Platform::from_str_exact(text).unwrap_or(Platform::Unknown)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn entry(seed: u8, name: &str) -> TrustEntry {
        TrustEntry {
            id: NodeId::from_bytes([seed; NodeId::LEN]),
            name: name.to_owned(),
            platform: Platform::Windows,
            paired_at_unix: 1_700_000_000,
        }
    }

    #[test]
    fn 文件不存在视为空库() {
        let dir = tempfile::tempdir().unwrap();
        let store = TrustStore::load(dir.path().join("trust.json")).unwrap();
        assert!(store.entries().is_empty());
        assert!(!store.is_trusted(NodeId::from_bytes([9; NodeId::LEN])));
    }

    #[test]
    fn 落盘重载后信任保持且条目字段完整() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trust.json");

        let mut store = TrustStore::load(&path).unwrap();
        store.trust(entry(1, "客厅 PC")).unwrap();
        store.trust(entry(2, "手机")).unwrap();

        let reloaded = TrustStore::load(&path).unwrap();
        assert!(reloaded.is_trusted(NodeId::from_bytes([1; NodeId::LEN])));
        assert!(reloaded.is_trusted(NodeId::from_bytes([2; NodeId::LEN])));
        assert_eq!(reloaded.entries(), store.entries());
        assert_eq!(reloaded.entries()[0].name, "客厅 PC");
        assert_eq!(reloaded.entries()[0].platform, Platform::Windows);
    }

    #[test]
    fn 同一指纹重复写入是更新而不是追加() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trust.json");
        let mut store = TrustStore::load(&path).unwrap();

        store.trust(entry(1, "旧名")).unwrap();
        store.trust(entry(1, "新名")).unwrap();

        assert_eq!(store.entries().len(), 1);
        assert_eq!(store.entries()[0].name, "新名");
        assert!(
            TrustStore::load(&path)
                .unwrap()
                .is_trusted(NodeId::from_bytes([1; NodeId::LEN]))
        );
    }

    #[test]
    fn 撤销后落盘为_false_且持久化() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trust.json");
        let mut store = TrustStore::load(&path).unwrap();
        store.trust(entry(1, "x")).unwrap();

        assert!(store.revoke(NodeId::from_bytes([1; NodeId::LEN])).unwrap());
        // 再撤销一次：什么都没删掉，返回 false。
        assert!(!store.revoke(NodeId::from_bytes([1; NodeId::LEN])).unwrap());

        let reloaded = TrustStore::load(&path).unwrap();
        assert!(!reloaded.is_trusted(NodeId::from_bytes([1; NodeId::LEN])));
        assert!(reloaded.entries().is_empty());
    }

    #[test]
    fn 损坏_json_返回错误而不是静默重置() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trust.json");

        for broken in [
            "{ 这不是 JSON".to_owned(),
            "".to_owned(),
            "{}".to_owned(),
            // 结构对但指纹非法。
            r#"{"version":1,"entries":[{"id":"zz","name":"x","platform":"win","paired_at_unix":1}]}"#
                .to_owned(),
            // 结构对但版本不认识。
            r#"{"version":99,"entries":[]}"#.to_owned(),
            // 重复指纹。
            format!(
                r#"{{"version":1,"entries":[{{"id":"{id}","name":"a","platform":"win","paired_at_unix":1}},{{"id":"{id}","name":"b","platform":"win","paired_at_unix":2}}]}}"#,
                id = "11".repeat(32)
            ),
        ] {
            fs::write(&path, &broken).unwrap();
            let outcome = TrustStore::load(&path);
            assert!(
                matches!(outcome, Err(IdentityError::TrustStore { .. })),
                "损坏内容 {broken:?} 竟然被接受了：{outcome:?}"
            );
        }
    }

    #[test]
    fn 未知平台保留记录而不是丢白名单() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trust.json");
        fs::write(
            &path,
            format!(
                r#"{{"version":1,"entries":[{{"id":"{id}","name":"未来设备","platform":"ios","paired_at_unix":1}}]}}"#,
                id = "ab".repeat(32)
            ),
        )
        .unwrap();

        let store = TrustStore::load(&path).unwrap();
        assert_eq!(store.entries().len(), 1);
        assert_eq!(store.entries()[0].platform, Platform::Unknown);
        assert!(store.is_trusted(NodeId::from_bytes([0xab; NodeId::LEN])));
    }
}
