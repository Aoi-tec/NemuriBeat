PRAGMA journal_mode=WAL;
CREATE TABLE IF NOT EXISTS raw_measurements (
 run_id TEXT NOT NULL, sequence_id INTEGER NOT NULL, timestamp INTEGER NOT NULL,
 sensor_id TEXT NOT NULL, heart_rate INTEGER NOT NULL, rr_ms TEXT NOT NULL, quality REAL NOT NULL,
 contact INTEGER, PRIMARY KEY(run_id,sequence_id));
CREATE INDEX IF NOT EXISTS raw_time ON raw_measurements(timestamp);
CREATE TABLE IF NOT EXISTS physio_features_1m (
 timestamp INTEGER NOT NULL, session_id TEXT NOT NULL DEFAULT '', hr_mean REAL, hr_min INTEGER,
 hr_max INTEGER, rmssd REAL, sdnn REAL, sleep_probability REAL, quality REAL NOT NULL,
 payload TEXT NOT NULL, PRIMARY KEY(timestamp,session_id));
CREATE TABLE IF NOT EXISTS sleep_sessions (
 session_id TEXT PRIMARY KEY, started_at INTEGER NOT NULL, estimated_sleep_at INTEGER,
 final_wake_at INTEGER, ended_at INTEGER, estimated_sleep_minutes REAL NOT NULL,
 avg_hr REAL, min_hr INTEGER, median_rmssd REAL, awakening_count INTEGER NOT NULL, payload TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY,value TEXT NOT NULL,updated_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS data_gaps (id INTEGER PRIMARY KEY, timestamp INTEGER NOT NULL, reason TEXT NOT NULL);

