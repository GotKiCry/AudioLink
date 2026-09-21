use audiolink_discovery::{Advertiser, Browser, DiscoveredHost};
use audiolink_engine::Engine;
use audiolink_proto::discovery::DiscoveryBeacon;

use crate::error::CommandError;

/// Advertisement follows the listening engine; browsing starts on first UI request.
pub struct LanDiscovery {
    _advertiser: Option<Advertiser>,
    browser: Option<Browser>,
    own_id: String,
}

impl LanDiscovery {
    pub fn start(engine: &Engine) -> Self {
        let info = engine.info();
        let own_id = info.id.short();
        let beacon = DiscoveryBeacon::new(
            own_id.clone(),
            info.name.clone(),
            info.platform,
            info.caps,
            engine.local_addr().port(),
        );
        let advertiser = Advertiser::start(beacon)
            .map_err(|error| {
                tracing::warn!(%error, "LAN advertisement failed to start");
            })
            .ok();
        Self {
            _advertiser: advertiser,
            browser: None,
            own_id,
        }
    }

    pub fn hosts(&mut self, refresh: bool) -> Result<Vec<DiscoveredHost>, CommandError> {
        let map_error = |error: std::io::Error| {
            CommandError::busy(
                "暂时无法搜索主机，可以手动输入地址连接",
                format!("LAN discovery: {error}"),
            )
        };
        if refresh {
            self.browser = None;
        }
        if self.browser.is_none() {
            self.browser = Some(Browser::start(Some(self.own_id.clone())).map_err(map_error)?);
        }
        let result = match &self.browser {
            Some(browser) => browser.hosts().map_err(map_error),
            None => Ok(Vec::new()),
        };
        if result.is_err() {
            self.browser = None;
        }
        result
    }
}
