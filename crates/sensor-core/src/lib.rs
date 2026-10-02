use async_trait::async_trait;
use shared_types::Measurement;
use tokio::sync::{mpsc, watch};
#[derive(Clone, Debug)]
pub struct Device {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Debug)]
pub enum SensorMessage {
    Connected(String),
    Disconnected(String),
    Measurement(Measurement),
}
#[async_trait]
pub trait SensorAdapter: Send + Sync {
    async fn scan(&self) -> anyhow::Result<Vec<Device>>;
    async fn run(
        &self,
        id: String,
        output: mpsc::Sender<SensorMessage>,
        stop: watch::Receiver<bool>,
    ) -> anyhow::Result<()>;
}
/// Bluetooth SIG Heart Rate Measurement (0x2A37). RR units are 1/1024 seconds.
pub fn parse_hr(data: &[u8], sensor_id: String, timestamp: i64) -> anyhow::Result<Measurement> {
    anyhow::ensure!(!data.is_empty(), "missing flags");
    let flags = data[0];
    let mut i = 1;
    let mut read = |n: usize| -> anyhow::Result<&[u8]> {
        anyhow::ensure!(i + n <= data.len(), "truncated Heart Rate Measurement");
        let part = &data[i..i + n];
        i += n;
        Ok(part)
    };
    let heart_rate = if flags & 1 != 0 {
        let b = read(2)?;
        u16::from_le_bytes([b[0], b[1]])
    } else {
        read(1)?[0] as u16
    };
    let contact = if flags & 4 != 0 {
        Some(flags & 2 != 0)
    } else {
        None
    };
    if flags & 8 != 0 {
        read(2)?;
    }
    let mut rr_ms = Vec::new();
    if flags & 16 != 0 {
        anyhow::ensure!(
            i < data.len() && (data.len() - i).is_multiple_of(2),
            "missing or truncated RR intervals"
        );
        for b in data[i..].as_chunks::<2>().0 {
            rr_ms.push(u16::from_le_bytes([b[0], b[1]]) as f64 * 1000.0 / 1024.0);
        }
    } else {
        anyhow::ensure!(i == data.len(), "unexpected bytes without RR flag");
    }
    Ok(Measurement {
        timestamp,
        sensor_id,
        source_timestamp: None,
        heart_rate,
        rr_ms,
        contact,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_multiple_rr_and_contact() {
        let m = parse_hr(&[0x16, 60, 0, 4, 0, 2], "x".into(), 1).unwrap();
        assert_eq!(m.rr_ms, vec![1000.0, 500.0]);
        assert_eq!(m.contact, Some(true));
    }
    #[test]
    fn parses_u16_energy() {
        assert_eq!(
            parse_hr(&[9, 44, 1, 9, 0], "x".into(), 0)
                .unwrap()
                .heart_rate,
            300
        );
    }
    #[test]
    fn rejects_truncation() {
        for b in [&[][..], &[1, 2], &[16, 60], &[16, 60, 1], &[8, 60, 1]] {
            assert!(parse_hr(b, "x".into(), 0).is_err());
        }
    }
}
