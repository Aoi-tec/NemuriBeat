"""Local JSON-lines worker. Official SleepECG features and pretrained ws-gru-mesa only."""
import contextlib
from datetime import datetime
import json
import math
import os
import sys

os.environ.setdefault('TF_CPP_MIN_LOG_LEVEL', '2')
os.environ.setdefault('TF_NUM_INTEROP_THREADS', '1')
os.environ.setdefault('TF_NUM_INTRAOP_THREADS', '1')
os.environ.setdefault('OMP_NUM_THREADS', '1')
MODEL = 'ws-gru-mesa'

def emit(value):
    print(json.dumps(value, allow_nan=False), flush=True)

def continuous_beats(measurements):
    """RR ordering is preserved; no HR-derived or missing beats are manufactured."""
    beats = [0.0]
    start = None
    last = None
    for m in measurements:
        rr = m.get('rr_ms', [])
        if not rr:
            continue
        timestamp = m['timestamp'] / 1000
        # Short/long genuine RR are kept as timing evidence. SleepECG's own
        # min_rri/max_rri preprocessing masks artifacts; do not remove elapsed time.
        bad = m.get('contact') is False or any(not math.isfinite(r) or not 0 < r <= 5000 for r in rr)
        gap = last is not None and (timestamp <= last or timestamp - last > 5)
        if bad:
            beats, start, last = [0.0], None, None
            continue
        if gap:
            beats, start = [0.0], None
        if start is None:
            start = timestamp - sum(rr) / 1000
        for interval in rr:
            beats.append(beats[-1] + interval / 1000)
        last = timestamp
    return start, beats

def infer(clf, measurements, np, sleepecg):
    start, beats = continuous_beats(measurements)
    params = clf.feature_extraction_params
    lookback = params['lookback']
    lookforward = params['lookforward']
    duration = beats[-1]
    # The shipped model needs 240 s history and 270 s future features.
    # Classify the newest *complete* 30 s epoch having both; never change those features.
    epoch = int((duration - lookforward) // 30) - 1
    if start is None or epoch * 30 < lookback:
        return {'ready': False, 'message': f'連続RRの蓄積中（必要 {lookback + lookforward + 30} 秒）'}
    record = sleepecg.SleepRecord(heartbeat_times=np.asarray(beats), sleep_stage_duration=30,
                                 recording_start_time=datetime.fromtimestamp(start))
    with contextlib.redirect_stdout(sys.stderr):
        probabilities = sleepecg.stage(clf, record, return_mode='prob')
    row = probabilities[epoch]
    # SleepECG wake-sleep: UNDEFINED=0, SLEEP=1, WAKE=2.
    if len(row) != 3 or not np.all(np.isfinite(row)) or int(np.argmax(row)) == 0:
        return {'ready': False, 'message': 'GRUの出力が判定不能です'}
    return {'ready': True, 'probability': float(row[1]),
            'evaluated_at': int((start + (epoch + 1) * 30) * 1000),
            'message': f'SleepECG公式GRU・約{lookforward // 60}〜{lookforward // 60 + 1}分遅延の暫定判定'}

def main():
    try:
        with contextlib.redirect_stdout(sys.stderr):
            import numpy as np
            import sleepecg
            clf = sleepecg.load_classifier(MODEL, 'SleepECG')
        if clf.stages_mode != 'wake-sleep':
            raise ValueError('unexpected stages')
        emit({'model': MODEL, 'ready': False, 'message': '公式GRUを読み込みました。RR蓄積待ち'})
    except Exception as exc:
        emit({'model': MODEL, 'ready': False, 'message': '公式GRUを読み込めません: ' + type(exc).__name__})
        return 1
    for line in sys.stdin:
        try:
            request = json.loads(line)
            result = infer(clf, request['measurements'], np, sleepecg)
            result['model'] = MODEL
            emit(result)
        except Exception as exc:
            emit({'model': MODEL, 'ready': False, 'message': 'GRU推論失敗: ' + type(exc).__name__})
    return 0

if __name__ == '__main__':
    sys.exit(main())
