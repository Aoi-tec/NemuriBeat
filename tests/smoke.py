"""Offline full-runtime checks for HR+RR recording and HR-only output. No HypeRate API key required."""
import concurrent.futures
import json
import os
from pathlib import Path
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

BINARY = str(Path(sys.argv[1]).resolve())
REPO = Path(__file__).resolve().parents[1]

def scenario(hr_only):
    with tempfile.TemporaryDirectory(prefix="nemuri-", dir="/tmp" if os.name == "posix" else None) as tmp:
        root = Path(tmp)
        personal = root / "Demo/PersonalData/nemuribeat.sqlite"
        cache = root / "Demo/Cache/derived.sqlite"
        personal.parent.mkdir(parents=True)
        db = sqlite3.connect(personal)
        db.executescript((REPO / "migrations/001_initial.sql").read_text())
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            api_port = reservation.getsockname()[1]
        osc = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        osc.bind(("127.0.0.1", 0))
        osc.settimeout(3)
        config = {"osc_enabled": True, "oscquery_enabled": False, "osc_port": osc.getsockname()[1], "api_enabled": True,
                  "api_port": api_port, "sleep_confirm_secs": 60}
        db.execute("INSERT INTO settings VALUES ('app',?,0)", [json.dumps(config)])
        db.commit()
        duration = 96
        cmd = [BINARY, "--demo-sleep", "--headless", "--seconds", str(duration), "--data-dir", str(root)]
        if hr_only:
            cmd += ["--demo-hr-only"]
        ipc_path = root / "state.sock"
        if os.name == "posix":
            cmd += ["--ipc", str(ipc_path)]
        process = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        base = f"http://127.0.0.1:{api_port}/api/v1"
        def get(path):
            with urllib.request.urlopen(base + path, timeout=3) as response:
                return json.load(response)
        try:
            deadline = time.monotonic() + 10
            while True:
                try:
                    latest = get("/physiology/latest")
                    break
                except urllib.error.URLError:
                    if process.poll() is not None or time.monotonic() > deadline:
                        raise
                    time.sleep(0.1)
            assert "hyperate_key" not in get("/settings")
            assert get("/sleep/sessions") == [], "Recording must not begin at application startup"
            assert osc.recv(4096).startswith(b"/avatar/parameters/VRCOSC/")
            if os.name == "posix":
                with socket.socket(socket.AF_UNIX) as client:
                    client.settimeout(3)
                    client.connect(str(ipc_path))
                    payload = json.loads(client.makefile("rb").readline())
                    assert "mode" in payload and "sleep_like" in payload["sleep"]
            db.execute("BEGIN IMMEDIATE")
            first = get("/physiology/latest")["sequence_id"]
            time.sleep(2.2)
            assert get("/physiology/latest")["sequence_id"] > first
            db.rollback()
            assert get("/sleep/status")["asleep"] is not True
            deadline = time.monotonic() + 90
            while True:
                state = get("/sleep/status")
                if state["asleep"]:
                    break
                assert time.monotonic() < deadline, state
                time.sleep(0.3)
            assert state["mode"] == ("hr_only" if hr_only else "hr_rr"), state
            assert state["estimated"] == hr_only
            assert state["measurement_available"] != hr_only
            if hr_only:
                s = get("/physiology/latest")
                assert s["last_rr_ms"] is None and s["rmssd_fast"] is None
                assert not s["rr_available"]
                assert get("/sleep/sessions") == []
            else:
                # Persistent state arrives asynchronously, but should be visible promptly.
                limit = time.monotonic() + 3
                while not (sessions := get("/sleep/sessions")):
                    assert time.monotonic() < limit
                    time.sleep(0.1)
                session = get("/sleep/sessions/" + sessions[0]["session_id"])
                assert session["session"]["estimated_sleep_minutes"] >= 1
                assert session["minutes"]
            try:
                get("/sleep/sessions/not-found")
                raise AssertionError("Expected 404")
            except urllib.error.HTTPError as error:
                assert error.code == 404
            stdout, stderr = process.communicate(timeout=45)
            assert process.returncode == 0, stderr
            assert "dropped=0" in stdout, stdout
            count, first, last = db.execute("SELECT count(*),min(sequence_id),max(sequence_id) FROM raw_measurements").fetchone()
            assert count >= 90 and count == last - first + 1
            sessions = db.execute("SELECT ended_at,estimated_sleep_minutes FROM sleep_sessions").fetchall()
            if hr_only:
                assert not sessions
                assert db.execute("SELECT count(*) FROM physio_features_1m").fetchone()[0] == 0
                assert all(json.loads(row[0]) == [] for row in db.execute("SELECT rr_ms FROM raw_measurements"))
            else:
                assert len(sessions) == 1 and sessions[0][0] is not None
                assert sessions[0][1] >= 1
            with sqlite3.connect(cache) as cache_db:
                assert cache_db.execute("SELECT count(*) FROM physio_features_1m").fetchone()[0] >= 2
                assert cache_db.execute("SELECT count(*) FROM raw_measurements").fetchone()[0] == 0
            if os.name == "posix":
                assert not ipc_path.exists()
            return f"PASS {'HR-only' if hr_only else 'HR+RR'}: {count} raw, {len(sessions)} sessions; API/IPC/OSC, DB-lock isolation, mode separation, flush"
        finally:
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=10)
            db.close()
            osc.close()

with concurrent.futures.ThreadPoolExecutor(max_workers=2) as executor:
    for result in executor.map(scenario, [False, True]):
        print(result, flush=True)
