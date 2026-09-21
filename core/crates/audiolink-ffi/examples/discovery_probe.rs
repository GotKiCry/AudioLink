//! Bounded device-side discovery check. Uses synthetic capture; never opens a microphone.
use audiolink_ffi::{EngineStartConfig, PcmPull, engine_start, engine_stop};
use std::time::Duration;

struct SilentSource;
impl PcmPull for SilentSource {
    fn read_pcm(&self, _max_samples: i32) -> Vec<f32> {
        Vec::new()
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap_or_else(|| "scan".into());
    let seconds = args
        .next()
        .unwrap_or_else(|| "15".into())
        .parse::<u64>()?
        .clamp(1, 60);
    if mode == "advertise" {
        let dir = args
            .next()
            .ok_or("advertise requires an identity directory argument")?;
        let status = engine_start(
            EngineStartConfig {
                node_name: "AudioLink discovery test (synthetic audio)".into(),
                data_dir: dir,
                listen_port: 58299,
                capabilities: 0,
            },
            None,
            Some(Box::new(SilentSource)),
        )
        .await?;
        println!("advertising {} at {}", status.id_short, status.addr);
        tokio::time::sleep(Duration::from_secs(seconds)).await;
        engine_stop().await?;
    } else if mode == "scan" {
        let browser = audiolink_ffi::discovery_bridge::DiscoveryBrowser::new()?;
        for _ in 0..seconds {
            for host in browser.hosts()? {
                println!("{} {} {}", host.id_short, host.addr, host.name);
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        browser.stop();
    } else {
        return Err("use scan or advertise".into());
    }
    Ok(())
}
