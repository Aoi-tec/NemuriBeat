"""End-to-end daily API and normal/faulted Python worker startup, with isolated DBs."""
import json
from pathlib import Path
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time
import urllib.request
import urllib.error

ROOT=Path(__file__).resolve().parents[1]
BINARY=str(Path(sys.argv[1]).resolve())
PYTHON=str(Path(sys.argv[2]).resolve())

def run(python, expect_model):
    with tempfile.TemporaryDirectory(prefix='nemuri-sleep-gru-') as directory:
        root=Path(directory)
        path=root/'PersonalData/nemuribeat.sqlite'
        path.parent.mkdir(parents=True)
        with socket.socket() as reservation:
            reservation.bind(('127.0.0.1',0));port=reservation.getsockname()[1]
        db=sqlite3.connect(path)
        db.executescript((ROOT/'migrations/001_initial.sql').read_text())
        db.execute("INSERT INTO settings VALUES ('app',?,0)",[json.dumps({'api_port':port,'sleep_python':python,'sleep_hmd_enabled':False})])
        for i,minutes in enumerate([60,90]):
            start=1790920000000+i*8*3600000
            session=dict(session_id=f'segment-{i}',sleep_day='2026-10-02',started_at=start,
                         estimated_sleep_at=start,final_wake_at=start+minutes*60000,ended_at=start+minutes*60000,
                         estimated_sleep_minutes=minutes,avg_hr=60,min_hr=50,median_rmssd=40,awakening_count=0,baseline_delta=None)
            values=[session[k] for k in ['session_id','started_at','estimated_sleep_at','final_wake_at','ended_at','estimated_sleep_minutes','avg_hr','min_hr','median_rmssd','awakening_count']]
            db.execute('INSERT INTO sleep_sessions VALUES (?,?,?,?,?,?,?,?,?,?,?)',values+[json.dumps(session)])
            features=dict(timestamp=start,hr_mean=60,hr_min=50,hr_max=70,rmssd=40,sdnn=30,pnn50=5,hr_slope=0,baseline_delta=0,quality=1,valid_rr=60)
            minute=dict(timestamp=start,features=features,sleep_probability=.9,session_id=session['session_id'],sample_count=60)
            db.execute('INSERT INTO physio_features_1m VALUES (?,?,?,?,?,?,?,?,?,?)',
                       [start,session['session_id'],60,50,70,40,30,.9,1,json.dumps(minute)])
        db.commit();db.close()
        process=subprocess.Popen([BINARY,'--headless','--api','--seconds','12','--data-dir',str(root)],stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
        base=f'http://127.0.0.1:{port}/api/v1'
        def get(url):
            with urllib.request.urlopen(base+url,timeout=2) as response:return json.load(response)
        try:
            deadline=time.monotonic()+8
            while True:
                try:
                    status=get('/sleep/status')
                    if expect_model and status['model']['model']!='ws-gru-mesa':
                        raise ConnectionError('model still loading')
                    if not expect_model and not status['model']['message']:
                        raise ConnectionError('worker fault not yet reported')
                    break
                except (OSError,ConnectionError):
                    assert time.monotonic()<deadline,'server/worker startup did not complete'
                    time.sleep(.1)
            assert status['tracking'] is False and status['probability'] is None,status
            assert status['hmd']['available'] is False
            days=get('/sleep/days');assert len(days)==1,days
            assert days[0]['summary']['estimated_sleep_minutes']==150
            assert len(days[0]['segments'])==2
            day=get('/sleep/days/2026-10-02');assert len(day['minutes'])==2,day
            assert len(get('/sleep/sessions'))==2,'raw segments must remain intact'
            try:get('/sleep/days/2099-01-01');raise AssertionError('unknown day must be 404')
            except urllib.error.HTTPError as error:assert error.code==404
            out,err=process.communicate(timeout=15)
            assert process.returncode==0,(out,err)
            assert 'Stopped cleanly.' in out,out
        finally:
            if process.poll() is None:process.terminate();process.communicate(timeout=5)
    print('PASS: daily API aggregation without filling gap, raw compatibility, unknown day, no-RR tracking=false, worker startup/failure and shutdown',expect_model)

run(PYTHON,True)
run('/nonexistent/nemuribeat-python',False)
