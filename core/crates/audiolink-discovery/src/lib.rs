//! IPv4 LAN host advertisements using the §9.2 UDP beacon format.
//! Discovery is independent of the audio engine and never establishes trust.
#![deny(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]

use audiolink_proto::discovery::{DiscoveryBeacon, MAX_BEACON_LEN};
use audiolink_types::{Caps, DEFAULT_DISCOVERY_PORT, PROTO_VERSION};
use serde::Serialize;
use socket2::{Domain, Protocol, Socket, Type};
use std::{
    collections::BTreeMap,
    io,
    net::{Ipv4Addr, SocketAddr, UdpSocket},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub const BEACON_INTERVAL: Duration = Duration::from_secs(3);
pub const HOST_TTL: Duration = Duration::from_secs(10);
const MAX_HOSTS: usize = 128;

/// An untrusted advertisement. `addr` always uses the datagram's source IP.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredHost {
    pub id_short: String,
    pub name: String,
    pub addr: String,
    pub platform: String,
    pub proto_version: u16,
    pub compatible: bool,
}

struct Worker {
    stopped: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}
impl Worker {
    fn spawn(name: &str, run: impl FnOnce(Arc<AtomicBool>) + Send + 'static) -> io::Result<Self> {
        let stopped = Arc::new(AtomicBool::new(false));
        let signal = Arc::clone(&stopped);
        let handle = thread::Builder::new()
            .name(name.into())
            .spawn(move || run(signal))?;
        Ok(Self {
            stopped,
            handle: Some(handle),
        })
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            handle.thread().unpark();
            let _ = handle.join();
        }
    }
}

/// Advertises on each IPv4 adapter, including Ethernet, Wi-Fi and hotspots.
/// Dropping the advertiser stops its worker. Network changes are picked up every tick.
pub struct Advertiser {
    _worker: Worker,
}
impl Advertiser {
    pub fn start(beacon: DiscoveryBeacon) -> io::Result<Self> {
        if beacon.port == 0 || !beacon.caps.contains(Caps::CAN_SEND) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "host must listen and support sending",
            ));
        }
        let packet = beacon
            .encode_to_vec()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error.to_string()))?;
        let worker = Worker::spawn("lan-advertise", move |stopped| {
            while !stopped.load(Ordering::Acquire) {
                if let Err(error) = advertise(&packet) {
                    tracing::debug!(%error, "LAN advertisement unavailable; retrying next tick");
                }
                thread::park_timeout(BEACON_INTERVAL);
            }
        })?;
        Ok(Self { _worker: worker })
    }
}
fn advertise(packet: &[u8]) -> io::Result<()> {
    for interface in if_addrs::get_if_addrs()? {
        let if_addrs::IfAddr::V4(addr) = interface.addr else {
            continue;
        };
        if addr.ip.is_loopback() || addr.ip.is_unspecified() || addr.ip.is_link_local() {
            continue;
        }
        let Some(broadcast) = addr.broadcast else {
            continue;
        };
        // Explicit adapter binding reaches both Wi-Fi and Windows mobile hotspots.
        let result = (|| -> io::Result<()> {
            let socket = UdpSocket::bind((addr.ip, 0))?;
            socket.set_broadcast(true)?;
            socket.set_write_timeout(Some(Duration::from_millis(100)))?;
            socket.send_to(packet, (broadcast, DEFAULT_DISCOVERY_PORT))?;
            Ok(())
        })();
        if let Err(error) = result {
            tracing::debug!(%error, ip = %addr.ip, "could not advertise on adapter");
        }
    }
    Ok(())
}

#[derive(Default)]
struct Hosts(BTreeMap<(String, String), (DiscoveredHost, Instant)>);
impl Hosts {
    fn prune(&mut self, now: Instant) {
        self.0
            .retain(|_, (_, seen)| now.saturating_duration_since(*seen) < HOST_TTL);
    }
    fn ingest(&mut self, packet: &[u8], source: SocketAddr, own_id: Option<&str>, now: Instant) {
        self.prune(now);
        let Ok(beacon) = DiscoveryBeacon::decode(packet) else {
            return;
        };
        let ip = source.ip();
        if ip.is_unspecified()
            || ip.is_multicast()
            || ip == Ipv4Addr::BROADCAST
            || beacon.port == 0
            || !beacon.caps.contains(Caps::CAN_SEND)
            || own_id.is_some_and(|id| id.eq_ignore_ascii_case(&beacon.id))
            || beacon.name.trim().is_empty()
        {
            return;
        }
        let host = DiscoveredHost {
            id_short: beacon.id,
            name: beacon
                .name
                .chars()
                .filter(|c| !c.is_control())
                .take(64)
                .collect(),
            addr: SocketAddr::new(ip, beacon.port).to_string(),
            platform: beacon.platform.as_str().into(),
            proto_version: beacon.proto,
            compatible: beacon.proto == PROTO_VERSION,
        };
        let key = (host.id_short.clone(), host.addr.clone());
        if self.0.len() < MAX_HOSTS || self.0.contains_key(&key) {
            self.0.insert(key, (host, now));
        }
    }
}

/// Passive discovery works before starting the audio engine. Drop releases the port
/// and joins the worker (one 200 ms receive timeout under normal conditions).
pub struct Browser {
    hosts: Arc<Mutex<Hosts>>,
    error: Arc<Mutex<Option<String>>>,
    _worker: Worker,
}
impl Browser {
    pub fn start(own_id: Option<String>) -> io::Result<Self> {
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
        socket.set_reuse_address(true)?;
        socket.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, DEFAULT_DISCOVERY_PORT)).into())?;
        Self::listen(socket.into(), own_id)
    }
    fn listen(socket: UdpSocket, own_id: Option<String>) -> io::Result<Self> {
        socket.set_read_timeout(Some(Duration::from_millis(200)))?;
        let hosts = Arc::new(Mutex::new(Hosts::default()));
        let error = Arc::new(Mutex::new(None));
        let table = Arc::clone(&hosts);
        let failure = Arc::clone(&error);
        let worker =
            Worker::spawn("lan-discover", move |stopped| {
                // An extra byte prevents accepting oversized datagrams truncated to a valid beacon.
                let mut buffer = [0u8; MAX_BEACON_LEN + 1];
                while !stopped.load(Ordering::Acquire) {
                    match socket.recv_from(&mut buffer) {
                        Ok((len, source)) => table
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .ingest(&buffer[..len], source, own_id.as_deref(), Instant::now()),
                        Err(error)
                            if matches!(
                                error.kind(),
                                io::ErrorKind::WouldBlock
                                    | io::ErrorKind::TimedOut
                                    | io::ErrorKind::Interrupted
                            ) => {}
                        // Windows reports an oversized datagram as WSAEMSGSIZE.
                        Err(error) if error.raw_os_error() == Some(10040) => {}
                        Err(error) => {
                            *failure.lock().unwrap_or_else(|e| e.into_inner()) =
                                Some(error.to_string());
                            break;
                        }
                    }
                }
            })?;
        Ok(Self {
            hosts,
            error,
            _worker: worker,
        })
    }
    pub fn hosts(&self) -> io::Result<Vec<DiscoveredHost>> {
        if let Some(error) = self
            .error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            return Err(io::Error::other(error.clone()));
        }
        let mut table = self.hosts.lock().unwrap_or_else(|e| e.into_inner());
        table.prune(Instant::now());
        let mut hosts: Vec<_> = table.0.values().map(|(host, _)| host.clone()).collect();
        hosts.sort_by(|a, b| (&a.name, &a.addr).cmp(&(&b.name, &b.addr)));
        Ok(hosts)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use audiolink_types::Platform;
    fn beacon() -> DiscoveryBeacon {
        DiscoveryBeacon::new(
            "0123456789abcdef".into(),
            "Studio".into(),
            Platform::Windows,
            Caps::CAN_SEND,
            58290,
        )
    }
    #[test]
    fn source_ip_and_announced_port_refresh_and_expire() {
        let mut hosts = Hosts::default();
        let now = Instant::now();
        let source = "192.168.1.10:49000".parse().unwrap();
        let bytes = beacon().encode_to_vec().unwrap();
        hosts.ingest(&bytes, source, None, now);
        hosts.ingest(&bytes, source, None, now + Duration::from_secs(3));
        assert_eq!(hosts.0.len(), 1);
        let host = &hosts.0.values().next().unwrap().0;
        assert_eq!(host.addr, "192.168.1.10:58290");
        assert!(host.compatible);
        hosts.prune(now + HOST_TTL);
        assert_eq!(hosts.0.len(), 1);
        hosts.prune(now + HOST_TTL + Duration::from_secs(3));
        assert!(hosts.0.is_empty());
    }
    #[test]
    fn ignores_self_receivers_malformed_and_zero_port() {
        let mut hosts = Hosts::default();
        let now = Instant::now();
        let source = "192.168.1.10:49000".parse().unwrap();
        let mut b = beacon();
        hosts.ingest(&b.encode_to_vec().unwrap(), source, Some(&b.id), now);
        b.caps = Caps::CAN_RECEIVE;
        hosts.ingest(&b.encode_to_vec().unwrap(), source, None, now);
        b.caps = Caps::CAN_SEND;
        b.port = 0;
        hosts.ingest(&b.encode_to_vec().unwrap(), source, None, now);
        hosts.ingest(b"not an AudioLink beacon", source, None, now);
        assert!(hosts.0.is_empty());
    }
    #[test]
    fn different_adapters_and_incompatible_versions_stay_visible() {
        let mut hosts = Hosts::default();
        let mut b = beacon();
        b.proto = PROTO_VERSION + 1;
        for ip in ["192.168.1.10:1", "192.168.137.1:2"] {
            hosts.ingest(
                &b.encode_to_vec().unwrap(),
                ip.parse().unwrap(),
                None,
                Instant::now(),
            );
        }
        assert_eq!(hosts.0.len(), 2);
        assert!(hosts.0.values().all(|(h, _)| !h.compatible));
    }
    #[test]
    fn host_table_is_bounded_and_known_hosts_can_refresh() {
        let mut hosts = Hosts::default();
        let now = Instant::now();
        for id in 0..MAX_HOSTS + 5 {
            let mut b = beacon();
            b.id = format!("{id:016x}");
            hosts.ingest(
                &b.encode_to_vec().unwrap(),
                "192.168.1.1:1".parse().unwrap(),
                None,
                now,
            );
        }
        assert_eq!(hosts.0.len(), MAX_HOSTS);
        let mut b = beacon();
        b.id = "0000000000000000".into();
        hosts.ingest(
            &b.encode_to_vec().unwrap(),
            "192.168.1.1:1".parse().unwrap(),
            None,
            now + Duration::from_secs(1),
        );
        hosts.prune(now + HOST_TTL);
        assert_eq!(hosts.0.len(), 1);
    }
    #[test]
    fn udp_receives_real_beacon_and_drop_releases_socket() {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = socket.local_addr().unwrap();
        let browser = Browser::listen(socket, None).unwrap();
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        sender
            .send_to(&beacon().encode_to_vec().unwrap(), addr)
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while browser.hosts().unwrap().is_empty() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(browser.hosts().unwrap()[0].addr, "127.0.0.1:58290");
        drop(browser);
        assert!(UdpSocket::bind(addr).is_ok());
    }

    #[test]
    #[ignore = "requires an active IPv4 LAN adapter and UDP broadcast permissions"]
    fn live_lan_advertisement_is_discovered() {
        let browser = Browser::start(None).unwrap();
        let mut b = beacon();
        b.id = "a11d15c0beac0001".into();
        b.name = "AudioLink discovery transport check".into();
        let id = b.id.clone();
        let _advertiser = Advertiser::start(b).unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if browser
                .hosts()
                .unwrap()
                .iter()
                .any(|host| host.id_short == id)
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "no advertisement received on any LAN adapter"
            );
            thread::sleep(Duration::from_millis(50));
        }
    }
}
