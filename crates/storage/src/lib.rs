use anyhow::{Context, Result};
mod daily;
pub use daily::{group_sleep_days, sleep_day};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use shared_types::*;
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, SyncSender, TrySendError},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Write {
    Raw {
        run_id: String,
        sequence: u64,
        measurement: Measurement,
        quality: f64,
    },
    Minute(FeatureMinute),
    PruneCache {
        before: i64,
    },
    Session(SleepSession),
    Settings(Settings),
    Gap {
        timestamp: i64,
        reason: String,
    },
}
#[derive(Default)]
pub struct Health {
    pub dropped: AtomicU64,
    pub written: AtomicU64,
    pub pending: AtomicU64,
    pub error: Mutex<Option<String>>,
}
#[derive(Clone)]
pub struct Storage {
    sender: SyncSender<Write>,
    pub health: Arc<Health>,
    stop: Arc<AtomicBool>,
}
pub struct Worker {
    handle: Option<thread::JoinHandle<Result<()>>>,
    stop: Arc<AtomicBool>,
}
pub fn open(path: &Path) -> Result<Connection> {
    let c = Connection::open(path)?;
    c.busy_timeout(Duration::from_millis(200))?;
    c.execute_batch(include_str!("../../../migrations/001_initial.sql"))?;
    let has_source = c
        .prepare("PRAGMA table_info(raw_measurements)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .iter()
        .any(|n| n == "source_timestamp");
    if !has_source {
        c.execute_batch("ALTER TABLE raw_measurements ADD COLUMN source_timestamp INTEGER;")?;
    }
    c.pragma_update(None, "user_version", 2)?;
    Ok(c)
}
impl Storage {
    pub fn start(path: PathBuf) -> Result<(Self, Worker)> {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        let mut c = open(&path)?;
        let spool = path.with_extension("pending.json");
        if spool.exists() {
            let commands: Vec<Write> = serde_json::from_slice(&std::fs::read(&spool)?)?;
            write_batch(&mut c, &commands)?;
            std::fs::remove_file(&spool)?;
        }
        let (sender, receiver) = mpsc::sync_channel(16384);
        let health = Arc::new(Health::default());
        let h = health.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let handle = thread::spawn(move || {
            let mut batch = Vec::new();
            let mut seen_drops = 0;
            loop {
                if batch.is_empty() {
                    match receiver.recv_timeout(Duration::from_millis(200)) {
                        Ok(w) => batch.push(w),
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(_) => {}
                    }
                }
                while batch.len() < 512 {
                    match receiver.try_recv() {
                        Ok(w) => batch.push(w),
                        Err(_) => break,
                    }
                }
                let dropped = h.dropped.load(Ordering::Relaxed);
                if dropped > seen_drops {
                    batch.push(Write::Gap {
                        timestamp: now_ms(),
                        reason: format!(
                            "Storage queue overflow: {} commands dropped",
                            dropped - seen_drops
                        ),
                    });
                    seen_drops = dropped;
                }
                if !batch.is_empty() {
                    match write_batch(&mut c, &batch) {
                        Ok(()) => {
                            h.written.fetch_add(batch.len() as u64, Ordering::Relaxed);
                            batch.clear();
                            *h.error.lock().unwrap() = None;
                        }
                        Err(e) => {
                            *h.error.lock().unwrap() = Some(e.to_string());
                            if stopping.load(Ordering::Relaxed) {
                                batch.extend(receiver.try_iter());
                                std::fs::write(&spool, serde_json::to_vec(&batch)?)
                                    .context("Could not preserve pending writes")?;
                                h.pending.store(batch.len() as u64, Ordering::Relaxed);
                                return Err(e.context("Pending writes saved for next launch"));
                            }
                            thread::sleep(Duration::from_millis(250));
                        }
                    }
                }
                if stopping.load(Ordering::Relaxed) && batch.is_empty() {
                    match receiver.try_recv() {
                        Ok(w) => batch.push(w),
                        Err(_) => break,
                    }
                }
            }
            Ok(())
        });
        Ok((
            Self {
                sender,
                health,
                stop: stop.clone(),
            },
            Worker {
                handle: Some(handle),
                stop,
            },
        ))
    }
    /// Bounded and nonblocking: disk failures must never stall acquisition.
    pub fn push(&self, w: Write) -> bool {
        if self.stop.load(Ordering::Relaxed) {
            return false;
        }
        match self.sender.try_send(w) {
            Ok(()) => true,
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                self.health.dropped.fetch_add(1, Ordering::Relaxed);
                false
            }
        }
    }
}
impl Worker {
    pub fn shutdown(mut self) -> Result<()> {
        self.stop.store(true, Ordering::Relaxed);
        self.handle
            .take()
            .unwrap()
            .join()
            .map_err(|_| anyhow::anyhow!("Storage worker panicked"))?
    }
}
fn write_batch(c: &mut Connection, batch: &[Write]) -> Result<()> {
    let tx = c.transaction()?;
    for w in batch {
        match w {
            Write::PruneCache { before } => {
                tx.execute(
                    "DELETE FROM physio_features_1m WHERE timestamp<?1",
                    [before],
                )?;
            }
            Write::Raw {
                run_id,
                sequence,
                measurement: m,
                quality,
            } => {
                tx.execute(
                    "INSERT OR IGNORE INTO raw_measurements(run_id,sequence_id,timestamp,sensor_id,heart_rate,rr_ms,quality,contact,source_timestamp) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                    params![
                        run_id,
                        sequence,
                        m.timestamp,
                        m.sensor_id,
                        m.heart_rate,
                        serde_json::to_string(&m.rr_ms)?,
                        quality,
                        m.contact,
                        m.source_timestamp
                    ],
                )?;
            }
            Write::Minute(m) => {
                let f = &m.features;
                tx.execute("INSERT OR REPLACE INTO physio_features_1m VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![m.timestamp,m.session_id.as_deref().unwrap_or(""),f.hr_mean,f.hr_min,f.hr_max,f.rmssd,f.sdnn,m.sleep_probability,f.quality,serde_json::to_string(m)?])?;
            }
            Write::Session(s) => {
                tx.execute("INSERT OR REPLACE INTO sleep_sessions VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",params![s.session_id,s.started_at,s.estimated_sleep_at,s.final_wake_at,s.ended_at,s.estimated_sleep_minutes,s.avg_hr,s.min_hr,s.median_rmssd,s.awakening_count,serde_json::to_string(s)?])?;
            }
            Write::Settings(s) => {
                tx.execute(
                    "INSERT OR REPLACE INTO settings VALUES ('app',?1,?2)",
                    params![serde_json::to_string(s)?, now_ms()],
                )?;
            }
            Write::Gap { timestamp, reason } => {
                tx.execute(
                    "INSERT INTO data_gaps(timestamp,reason) VALUES (?1,?2)",
                    params![timestamp, reason],
                )?;
            }
        }
    }
    tx.commit()?;
    Ok(())
}
fn read_connection(path: &Path) -> Result<Connection> {
    let c = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    c.busy_timeout(Duration::from_millis(200))?;
    Ok(c)
}
pub fn settings(path: &Path) -> Result<Settings> {
    let c = read_connection(path)?;
    let value = c.query_row("SELECT value FROM settings WHERE key='app'", [], |r| {
        r.get::<_, String>(0)
    });
    match value {
        Ok(s) => Ok(serde_json::from_str(&s)?),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(Settings::default()),
        Err(e) => Err(e.into()),
    }
}
pub fn sessions(path: &Path, since: i64) -> Result<Vec<SleepSession>> {
    let c = read_connection(path)?;
    let mut stmt=c.prepare("SELECT payload FROM sleep_sessions WHERE started_at>=?1 ORDER BY started_at DESC LIMIT 10000")?;
    let rows = stmt.query_map([since], |r| r.get::<_, String>(0))?;
    rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
}
pub fn sleep_days(path: &Path) -> Result<Vec<DailySleep>> {
    let hour = settings(path)?.sleep_day_start_hour;
    Ok(group_sleep_days(&sessions(path, 0)?, hour))
}
pub fn day_history(path: &Path, day: &DailySleep) -> Result<Vec<FeatureMinute>> {
    let mut values = Vec::new();
    for segment in &day.segments {
        values.extend(history(path, 0, Some(&segment.session_id))?);
    }
    values.sort_by_key(|m| m.timestamp);
    // A minute reattached during sleep confirmation must not appear twice in a day.
    values.dedup_by_key(|m| m.timestamp);
    Ok(values)
}
pub fn history(path: &Path, since: i64, session: Option<&str>) -> Result<Vec<FeatureMinute>> {
    let c = read_connection(path)?;
    let mut stmt=c.prepare("SELECT payload FROM physio_features_1m WHERE timestamp>=?1 AND (?2 IS NULL OR session_id=?2) ORDER BY timestamp DESC LIMIT 20160")?;
    let rows = stmt.query_map(params![since, session], |r| r.get::<_, String>(0))?;
    let mut values: Vec<FeatureMinute> = rows
        .map(|r| Ok(serde_json::from_str(&r?)?))
        .collect::<Result<_>>()?;
    values.reverse();
    Ok(values)
}
pub fn baseline(path: &Path, days: u32, now: i64) -> Result<Baseline> {
    let c = read_connection(path)?;
    let (hr,rmssd,n)=c.query_row("SELECT AVG(hr_mean),AVG(rmssd),COUNT(*) FROM physio_features_1m WHERE timestamp>=?1 AND timestamp<?2 AND quality>=0.8 AND hr_mean IS NOT NULL AND rmssd IS NOT NULL",params![now-days as i64*86400000,now],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
    Ok(Baseline {
        resting_hr: hr,
        rmssd,
        sample_minutes: n,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raw_preserves_all_rr_and_deduplicates_retry() {
        let mut c = open(Path::new(":memory:")).unwrap();
        let w = Write::Raw {
            run_id: "r".into(),
            sequence: 1,
            measurement: Measurement {
                timestamp: 1,
                sensor_id: "s".into(),
                source_timestamp: None,
                heart_rate: 60,
                rr_ms: vec![1000.0, 1020.0],
                contact: None,
            },
            quality: 1.0,
        };
        write_batch(&mut c, &[w.clone(), w]).unwrap();
        let (count, rr): (i32, String) = c
            .query_row("SELECT COUNT(*),rr_ms FROM raw_measurements", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(rr, "[1000.0,1020.0]");
    }
    #[test]
    fn shutdown_flushes() {
        let path = std::env::temp_dir().join(format!("physio-{}.sqlite", now_ms()));
        let (s, w) = Storage::start(path.clone()).unwrap();
        for _ in 0..100 {
            assert!(s.push(Write::Gap {
                timestamp: 1,
                reason: "test".into()
            }));
        }
        w.shutdown().unwrap();
        let c = open(&path).unwrap();
        assert_eq!(
            c.query_row("SELECT count(*) FROM data_gaps", [], |r| r.get::<_, u32>(0))
                .unwrap(),
            100
        );
        drop(c);
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod resilience_tests {
    use super::*;
    #[test]
    fn locked_database_retries_and_reports_overflow() {
        let path = std::env::temp_dir().join(format!("physio-lock-{}.sqlite", now_ms()));
        let (store, worker) = Storage::start(path.clone()).unwrap();
        let blocker = open(&path).unwrap();
        blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
        for _ in 0..20000 {
            store.push(Write::Gap {
                timestamp: 1,
                reason: "queued".into(),
            });
        }
        assert!(store.health.dropped.load(Ordering::Relaxed) > 0);
        blocker.execute_batch("ROLLBACK").unwrap();
        worker.shutdown().unwrap();
        let n: i64 = blocker
            .query_row(
                "SELECT COUNT(*) FROM data_gaps WHERE reason LIKE 'Storage queue overflow%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(n > 0);
        assert!(store.health.written.load(Ordering::Relaxed) > 0);
        drop(blocker);
        let _ = std::fs::remove_file(path);
    }
    #[test]
    fn pending_spool_replays_on_restart() {
        let path = std::env::temp_dir().join(format!("physio-replay-{}.sqlite", now_ms()));
        std::fs::write(
            path.with_extension("pending.json"),
            serde_json::to_vec(&vec![Write::Gap {
                timestamp: 42,
                reason: "recovered".into(),
            }])
            .unwrap(),
        )
        .unwrap();
        let (_store, worker) = Storage::start(path.clone()).unwrap();
        worker.shutdown().unwrap();
        let c = open(&path).unwrap();
        assert_eq!(
            c.query_row(
                "SELECT COUNT(*) FROM data_gaps WHERE timestamp=42",
                [],
                |r| r.get::<_, u32>(0)
            )
            .unwrap(),
            1
        );
        assert!(!path.with_extension("pending.json").exists());
        drop(c);
        let _ = std::fs::remove_file(path);
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            match handle.join() {
                Ok(Ok(())) => {}
                Ok(Err(e)) => eprintln!("Storage shutdown: {e:#}"),
                Err(_) => eprintln!("Storage worker panicked during shutdown"),
            }
        }
    }
}

/// Copy a legacy live WAL database without modifying it. Never overwrite a destination.
pub fn migrate_legacy(source: &Path, destination: &Path) -> Result<()> {
    anyhow::ensure!(
        !destination.exists(),
        "Migration destination already exists"
    );
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let pending_source = source.with_extension("pending.json");
    let source = read_connection(source)?;
    let temporary = destination.with_extension("migrating.sqlite");
    anyhow::ensure!(
        !temporary.exists(),
        "An unfinished migration exists: {}",
        temporary.display()
    );
    let mut target = Connection::open(&temporary)?;
    {
        let backup = rusqlite::backup::Backup::new(&source, &mut target)?;
        backup.run_to_completion(128, Duration::from_millis(10), None)?;
    }
    drop(target);
    if pending_source.exists() {
        let pending_target = destination.with_extension("pending.json");
        anyhow::ensure!(
            !pending_target.exists(),
            "Pending migration destination already exists"
        );
        std::fs::copy(pending_source, pending_target)?;
    }
    std::fs::rename(temporary, destination)?;
    Ok(())
}

#[cfg(test)]
mod migration_tests {
    use super::*;
    #[test]
    fn migration_preserves_live_wal_and_original() {
        let root = std::env::temp_dir().join(format!("nemuri-migration-{}", now_ms()));
        std::fs::create_dir_all(&root).unwrap();
        let old = root.join("old.sqlite");
        let new = root.join("new.sqlite");
        let live = Connection::open(&old).unwrap();
        live.execute_batch(include_str!("../../../migrations/001_initial.sql"))
            .unwrap();
        live.execute(
            "INSERT INTO raw_measurements VALUES ('run',1,1000,'sensor',60,'[1000]',1,NULL)",
            [],
        )
        .unwrap();
        std::fs::write(old.with_extension("pending.json"), b"[]").unwrap();
        migrate_legacy(&old, &new).unwrap();
        let db = open(&new).unwrap();
        assert_eq!(
            db.query_row("SELECT heart_rate FROM raw_measurements", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            60
        );
        assert!(old.exists());
        assert!(old.with_extension("pending.json").exists());
        assert!(new.with_extension("pending.json").exists());
        assert!(migrate_legacy(&old, &new).is_err());
        drop(db);
        drop(live);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn pruning_cache_never_deletes_raw_or_sessions() {
        let mut c = open(Path::new(":memory:")).unwrap();
        let m = FeatureMinute {
            timestamp: 100,
            features: Features::default(),
            sleep_probability: None,
            session_id: None,
            sample_count: 1,
        };
        write_batch(
            &mut c,
            &[
                Write::Minute(m),
                Write::Raw {
                    run_id: "r".into(),
                    sequence: 1,
                    measurement: Measurement {
                        timestamp: 100,
                        source_timestamp: Some(90),
                        sensor_id: "HypeRate".into(),
                        heart_rate: 60,
                        rr_ms: vec![],
                        contact: None,
                    },
                    quality: 1.0,
                },
                Write::PruneCache { before: 200 },
            ],
        )
        .unwrap();
        assert_eq!(
            c.query_row("SELECT count(*) FROM physio_features_1m", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            c.query_row("SELECT source_timestamp FROM raw_measurements", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            90
        );
    }
}
