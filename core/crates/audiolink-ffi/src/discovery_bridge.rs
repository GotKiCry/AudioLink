//! LAN discovery can run before the audio service starts.
use crate::FfiError;
use audiolink_discovery::Browser;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, uniffi::Record)]
pub struct DiscoveredHost {
    pub id_short: String,
    pub name: String,
    pub addr: String,
    pub platform: String,
    pub proto_version: u16,
    pub compatible: bool,
}

#[derive(uniffi::Object)]
pub struct DiscoveryBrowser {
    browser: Mutex<Option<Browser>>,
}

#[uniffi::export]
impl DiscoveryBrowser {
    #[uniffi::constructor]
    pub fn new() -> Result<Arc<Self>, FfiError> {
        let browser = Browser::start(None)
            .map_err(|e| FfiError::invalid_argument(format!("discovery: {e}")))?;
        Ok(Arc::new(Self {
            browser: Mutex::new(Some(browser)),
        }))
    }

    pub fn hosts(&self) -> Result<Vec<DiscoveredHost>, FfiError> {
        let guard = self.browser.lock().unwrap_or_else(|e| e.into_inner());
        let browser = guard
            .as_ref()
            .ok_or_else(|| FfiError::invalid_argument("discovery is closed"))?;
        browser
            .hosts()
            .map_err(|e| FfiError::invalid_argument(format!("discovery: {e}")))
            .map(|hosts| {
                hosts
                    .into_iter()
                    .map(|host| DiscoveredHost {
                        id_short: host.id_short,
                        name: host.name,
                        addr: host.addr,
                        platform: host.platform,
                        proto_version: host.proto_version,
                        compatible: host.compatible,
                    })
                    .collect()
            })
    }

    /// Releases the socket immediately when the receiving screen leaves the foreground.
    pub fn stop(&self) {
        self.browser
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
    }
}
