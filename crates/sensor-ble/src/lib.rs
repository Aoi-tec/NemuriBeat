use anyhow::{Context, Result};
use async_trait::async_trait;
use btleplug::{
    api::{Central, Manager as _, Peripheral as _, ScanFilter},
    platform::{Adapter, Manager},
};
use futures::StreamExt;
use sensor_core::{parse_hr, Device, SensorAdapter, SensorMessage};
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use uuid::Uuid;
const HR: Uuid = Uuid::from_u128(0x00002a37_0000_1000_8000_00805f9b34fb);
pub struct GenericBleHRAdapter {
    adapter: Adapter,
}
impl GenericBleHRAdapter {
    pub async fn new() -> Result<Self> {
        let adapter = Manager::new()
            .await?
            .adapters()
            .await?
            .into_iter()
            .next()
            .context("No Bluetooth adapter")?;
        Ok(Self { adapter })
    }
}
#[async_trait]
impl SensorAdapter for GenericBleHRAdapter {
    async fn scan(&self) -> Result<Vec<Device>> {
        self.adapter.start_scan(ScanFilter::default()).await?;
        tokio::time::sleep(Duration::from_secs(4)).await;
        self.adapter.stop_scan().await?;
        let mut found = Vec::new();
        for p in self.adapter.peripherals().await? {
            if let Some(props) = p.properties().await? {
                found.push(Device {
                    id: p.id().to_string(),
                    name: props
                        .local_name
                        .unwrap_or_else(|| "Unnamed BLE device".into()),
                });
            }
        }
        found.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(found)
    }
    async fn run(
        &self,
        id: String,
        output: mpsc::Sender<SensorMessage>,
        mut stop: watch::Receiver<bool>,
    ) -> Result<()> {
        let p = self
            .adapter
            .peripherals()
            .await?
            .into_iter()
            .find(|p| p.id().to_string() == id)
            .context("Scan and select a device first")?;
        loop {
            if *stop.borrow() {
                break;
            }
            let mut notify_stop = stop.clone();
            let attempt = async {
                tokio::time::timeout(Duration::from_secs(15), p.connect()).await??;
                p.discover_services().await?;
                let c = p
                    .characteristics()
                    .into_iter()
                    .find(|c| c.uuid == HR)
                    .context("Device has no Heart Rate Measurement service")?;
                let mut stream = p.notifications().await?;
                p.subscribe(&c).await?;
                output.send(SensorMessage::Connected(id.clone())).await?;
                loop {
                    tokio::select! {
                        _=notify_stop.changed()=>break,
                        value=tokio::time::timeout(Duration::from_secs(10),stream.next())=> {
                            let n=value.context("Heart rate notifications timed out")?.context("BLE notification stream ended")?;
                            if n.uuid == HR {
                                let m=parse_hr(&n.value,id.clone(),shared_types::now_ms())?;
                                output.send(SensorMessage::Measurement(m)).await?;
                            }
                        }
                    }
                }
                Ok::<_, anyhow::Error>(())
            };
            let result = tokio::select! { r=attempt=>r, _=stop.changed()=>Ok(()) };
            let _ = tokio::time::timeout(Duration::from_secs(3), p.disconnect()).await;
            let reason = result
                .err()
                .map(|e| e.to_string())
                .unwrap_or_else(|| "Disconnected".into());
            if output
                .send(SensorMessage::Disconnected(reason))
                .await
                .is_err()
            {
                break;
            }
            if *stop.borrow() {
                break;
            }
            tokio::select! { _=tokio::time::sleep(Duration::from_secs(3))=>{}, _=stop.changed()=>break }
        }
        Ok(())
    }
}
/// Coospo and Polar devices using standard 0x180D/0x2A37 need no manufacturer-specific parser.
pub type CoospoBleAdapter = GenericBleHRAdapter;
