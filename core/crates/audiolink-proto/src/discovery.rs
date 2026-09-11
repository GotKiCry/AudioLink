//! 发现报文编解码（`docs/03-protocol.md` §9）：UDP 广播兜底报文 + mDNS TXT 字段。
//!
//! 本模块**只做编解码与格式校验，不做任何网络 I/O** —— 实际收发与 mDNS 注册属 `audiolink-discovery`。

use audiolink_types::{AudioLinkError, Caps, DISCOVERY_VERSION, PROTO_VERSION, Platform};
use serde::{Deserialize, Serialize};

use crate::fixed_bytes;

/// 广播报文魔数（9 B ASCII，§9.2）。
pub const MAGIC: &[u8; 9] = b"AUDIOLINK";

/// 广播报文固定头长度：`magic`(9) + `ver`(1) + `len`(2)。
pub const HEADER_LEN: usize = 12;

/// 广播 JSON 载荷长度上限（§9.2）。
pub const MAX_JSON_LEN: usize = 1024;

/// 单条广播报文总长上限（§9.2）。
pub const MAX_BEACON_LEN: usize = HEADER_LEN + MAX_JSON_LEN;

/// UDP 广播兜底发现报文（§9.2）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryBeacon {
    /// 发现协议版本（当前 `1`）。
    pub ver: u8,
    /// ALP 版本（`0x0201`）。
    pub proto: u16,
    /// 节点指纹（`SHA-256(证书 DER)`）前 8 字节，16 位小写 hex。
    pub id: String,
    /// 显示名（UTF-8，可含非 ASCII）。
    pub name: String,
    /// 平台（文本载体只支持 `win` / `android`）。
    pub platform: Platform,
    /// 能力位图。
    pub caps: Caps,
    /// 对端 QUIC 监听端口。
    pub port: u16,
}

impl DiscoveryBeacon {
    /// 以当前协议版本构造。
    pub fn new(id: String, name: String, platform: Platform, caps: Caps, port: u16) -> Self {
        Self {
            ver: DISCOVERY_VERSION,
            proto: PROTO_VERSION,
            id,
            name,
            platform,
            caps,
            port,
        }
    }

    /// 编码为广播报文：`magic` + `ver` + `len`(u16 LE) + JSON。
    pub fn encode_to_vec(&self) -> Result<Vec<u8>, AudioLinkError> {
        let json = self.json_payload()?;
        let len = u16::try_from(json.len())
            .map_err(|_| AudioLinkError::bad_request("discovery JSON length overflow"))?;
        let mut out = Vec::with_capacity(HEADER_LEN + json.len());
        out.extend_from_slice(MAGIC);
        out.push(self.ver);
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&json);
        Ok(out)
    }

    /// 解析广播报文：`magic` / 长度 / `ver` / JSON / 字段格式全部校验（§9.2）。
    pub fn decode(buf: &[u8]) -> Result<Self, AudioLinkError> {
        if buf.len() > MAX_BEACON_LEN {
            return Err(AudioLinkError::bad_request(
                "discovery packet exceeds 1036 B",
            ));
        }
        if buf.get(..MAGIC.len()) != Some(MAGIC.as_slice()) {
            return Err(AudioLinkError::bad_request("discovery magic != AUDIOLINK"));
        }
        let truncated = "discovery packet truncated";
        let [ver] = fixed_bytes::<1>(buf, MAGIC.len(), truncated)?;
        let len = u16::from_le_bytes(fixed_bytes::<2>(buf, MAGIC.len() + 1, truncated)?) as usize;
        if ver != DISCOVERY_VERSION {
            return Err(AudioLinkError::bad_request("discovery ver != 1"));
        }
        let json = buf
            .get(HEADER_LEN..)
            .ok_or(AudioLinkError::bad_request(truncated))?;
        if len > MAX_JSON_LEN {
            return Err(AudioLinkError::bad_request("discovery len > 1024"));
        }
        if len != json.len() {
            return Err(AudioLinkError::bad_request(
                "discovery len must equal remaining bytes",
            ));
        }
        Self::from_json_payload(json)
    }

    /// JSON 载荷（字段名与格式严格按 §9.2：`proto` 带 `0x` 前缀、`caps` 无前缀小写 hex）。
    pub fn json_payload(&self) -> Result<Vec<u8>, AudioLinkError> {
        self.validate()?;
        let dto = BeaconJson {
            v: self.ver,
            proto: format!("0x{:04x}", self.proto),
            id: self.id.to_ascii_lowercase(),
            name: self.name.clone(),
            platform: self.platform.as_str().to_string(),
            caps: self.caps.to_hex(),
            port: self.port,
        };
        let json = serde_json::to_vec(&dto)
            .map_err(|_| AudioLinkError::bad_request("discovery JSON encode failed"))?;
        if json.len() > MAX_JSON_LEN {
            return Err(AudioLinkError::bad_request("discovery JSON exceeds 1024 B"));
        }
        Ok(json)
    }

    /// 解析 JSON 载荷并规范化字段（大小写 / hex 格式）。
    pub fn from_json_payload(json: &[u8]) -> Result<Self, AudioLinkError> {
        if json.len() > MAX_JSON_LEN {
            return Err(AudioLinkError::bad_request("discovery JSON exceeds 1024 B"));
        }
        let dto: BeaconJson = serde_json::from_slice(json)
            .map_err(|_| AudioLinkError::bad_request("discovery JSON invalid"))?;
        if dto.v != DISCOVERY_VERSION {
            return Err(AudioLinkError::bad_request("discovery v != 1"));
        }
        let proto = parse_proto_hex(&dto.proto)?;
        validate_fingerprint_id(&dto.id)?;
        let platform = Platform::from_str_exact(&dto.platform)
            .ok_or(AudioLinkError::bad_request("unknown discovery platform"))?;
        let caps = Caps::from_hex(&dto.caps).ok_or(AudioLinkError::bad_request(
            "discovery caps is not valid hex",
        ))?;
        Ok(Self {
            ver: dto.v,
            proto,
            id: dto.id.to_ascii_lowercase(),
            name: dto.name,
            platform,
            caps,
            port: dto.port,
        })
    }

    /// 字段格式校验（§9.2）。
    fn validate(&self) -> Result<(), AudioLinkError> {
        if self.ver != DISCOVERY_VERSION {
            return Err(AudioLinkError::bad_request("discovery ver != 1"));
        }
        validate_fingerprint_id(&self.id)?;
        match self.platform {
            Platform::Windows | Platform::Android => Ok(()),
            Platform::Unknown => Err(AudioLinkError::bad_request(
                "discovery platform must be win / android",
            )),
        }
    }
}

/// mDNS TXT 记录字段（§9.1 / §9.2 字段格式表）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryTxt {
    /// 发现协议版本（当前 `1`）。
    pub v: u8,
    /// ALP 版本（`0x0201`）。
    pub proto: u16,
    /// 节点指纹前 8 字节（16 位小写 hex）。
    pub id: String,
    /// 显示名（UTF-8）。
    pub name: String,
    /// 平台（只支持 `win` / `android`）。
    pub platform: Platform,
    /// 能力位图。
    pub caps: Caps,
    /// 是否已与「我」配对（仅 UI 提示，不携带敏感信息）。
    pub paired: bool,
}

impl DiscoveryTxt {
    /// 以当前协议版本构造。
    pub fn new(id: String, name: String, platform: Platform, caps: Caps, paired: bool) -> Self {
        Self {
            v: DISCOVERY_VERSION,
            proto: PROTO_VERSION,
            id,
            name,
            platform,
            caps,
            paired,
        }
    }

    /// 编码为 TXT 键值对（顺序固定，便于抓包比对与测试）。
    pub fn to_pairs(&self) -> Vec<(String, String)> {
        vec![
            ("v".to_string(), self.v.to_string()),
            ("proto".to_string(), format!("0x{:04x}", self.proto)),
            ("id".to_string(), self.id.to_ascii_lowercase()),
            ("name".to_string(), self.name.clone()),
            ("platform".to_string(), self.platform.as_str().to_string()),
            ("caps".to_string(), self.caps.to_hex()),
            (
                "paired".to_string(),
                if self.paired { "1" } else { "0" }.to_string(),
            ),
        ]
    }

    /// 解析 TXT 键值对：未知 Key 忽略（向前兼容），缺必需 Key 或格式非法 → `BadRequest`（§9.2）。
    pub fn from_pairs(pairs: &[(String, String)]) -> Result<Self, AudioLinkError> {
        let mut v: Option<u8> = None;
        let mut proto: Option<u16> = None;
        let mut id: Option<String> = None;
        let mut name: Option<String> = None;
        let mut platform: Option<Platform> = None;
        let mut caps: Option<Caps> = None;
        let mut paired = false;

        for (key, value) in pairs {
            match key.as_str() {
                "v" => {
                    // §9.2：只接受 ASCII 数字（`u8::parse` 会放行 `+1`，这里显式收紧）
                    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                        return Err(AudioLinkError::bad_request(
                            "discovery v must be an ASCII integer",
                        ));
                    }
                    let parsed = value.parse::<u8>().map_err(|_| {
                        AudioLinkError::bad_request("discovery v must be an integer")
                    })?;
                    if parsed != DISCOVERY_VERSION {
                        return Err(AudioLinkError::bad_request("discovery v != 1"));
                    }
                    v = Some(parsed);
                }
                "proto" => proto = Some(parse_proto_hex(value)?),
                "id" => {
                    validate_fingerprint_id(value)?;
                    id = Some(value.to_ascii_lowercase());
                }
                "name" => name = Some(value.clone()),
                "platform" => {
                    platform = Some(
                        Platform::from_str_exact(value)
                            .ok_or(AudioLinkError::bad_request("unknown discovery platform"))?,
                    );
                }
                "caps" => {
                    caps = Some(Caps::from_hex(value).ok_or(AudioLinkError::bad_request(
                        "discovery caps is not valid hex",
                    ))?);
                }
                "paired" => {
                    paired = match value.as_str() {
                        "0" => false,
                        "1" => true,
                        _ => {
                            return Err(AudioLinkError::bad_request(
                                "discovery paired must be 0 or 1",
                            ));
                        }
                    };
                }
                // 未知 Key 一律忽略（§9.2 向前兼容）
                _ => {}
            }
        }

        let missing = |key: &'static str| {
            AudioLinkError::bad_request_owned(format!("discovery TXT missing key: {key}"))
        };
        Ok(Self {
            v: v.ok_or_else(|| missing("v"))?,
            proto: proto.ok_or_else(|| missing("proto"))?,
            id: id.ok_or_else(|| missing("id"))?,
            name: name.ok_or_else(|| missing("name"))?,
            platform: platform.ok_or_else(|| missing("platform"))?,
            caps: caps.ok_or_else(|| missing("caps"))?,
            paired,
        })
    }
}

/// 广播 JSON 的 DTO（字段名与类型严格对齐 §9.2；未知字段忽略以保持向前兼容）。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct BeaconJson {
    v: u8,
    proto: String,
    id: String,
    name: String,
    platform: String,
    caps: String,
    port: u16,
}

/// 校验指纹短码：16 位 hex（大小写均可，§9.2）。
fn validate_fingerprint_id(id: &str) -> Result<(), AudioLinkError> {
    if id.len() != 16 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(AudioLinkError::bad_request(
            "discovery id must be 16 hex chars",
        ));
    }
    Ok(())
}

/// 解析 `proto` 字段：带 `0x` 前缀的 4 位 hex（§9.2）。
fn parse_proto_hex(text: &str) -> Result<u16, AudioLinkError> {
    let hex = text.strip_prefix("0x").ok_or(AudioLinkError::bad_request(
        "discovery proto must start with 0x",
    ))?;
    if hex.len() != 4 {
        return Err(AudioLinkError::bad_request(
            "discovery proto must be 4 hex digits",
        ));
    }
    u16::from_str_radix(hex, 16)
        .map_err(|_| AudioLinkError::bad_request("discovery proto is not valid hex"))
}
