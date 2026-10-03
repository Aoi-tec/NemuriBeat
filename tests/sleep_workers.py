"""Protocol/input integrity tests. Optional --model runs actual official GRU weights."""
import importlib.util
import json
import math
from pathlib import Path
import subprocess
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('gru', ROOT / 'apps/desktop/python/sleep_gru.py')
gru = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gru)
spec = importlib.util.spec_from_file_location('hmd', ROOT / 'apps/desktop/python/hmd_openvr.py')
hmd = importlib.util.module_from_spec(spec)
spec.loader.exec_module(hmd)

class InputTests(unittest.TestCase):
    def test_no_rr_does_not_manufacture_beats(self):
        start, beats=gru.continuous_beats([{'timestamp':1000,'heart_rate':60,'rr_ms':[]}])
        self.assertIsNone(start);self.assertEqual(beats,[0.0])
    def test_gap_contact_and_invalid_rr_reset_sequence(self):
        for middle in [{'timestamp':9000,'rr_ms':[1000]}, {'timestamp':2000,'rr_ms':[-1]}, {'timestamp':2000,'rr_ms':[1000],'contact':False}]:
            start, beats=gru.continuous_beats([{'timestamp':1000,'rr_ms':[1000]},middle,{'timestamp':10000,'rr_ms':[1000]}])
            self.assertLessEqual(len(beats),3)
            self.assertGreaterEqual(start,8.0)
    def test_genuine_multiple_rr_are_preserved(self):
        _, beats=gru.continuous_beats([{'timestamp':3000,'rr_ms':[900,1100]}])
        self.assertEqual(beats,[0.0,0.9,2.0])
    def test_artifact_is_left_for_official_preprocessing_without_shortening_time(self):
        _, beats=gru.continuous_beats([{'timestamp':2100,'rr_ms':[1000,100,1000]}])
        self.assertEqual(beats,[0.0,1.0,1.1,2.1])
    def test_proximity_unknown_is_not_assumed_worn(self):
        from types import SimpleNamespace as S
        vr=S(k_unTrackedDeviceIndex_Hmd=0,Prop_ContainsProximitySensor_Bool=1,k_EButton_ProximitySensor=31)
        system=S(getBoolTrackedDeviceProperty=lambda *_:False)
        self.assertIsNone(hmd.worn_state(vr,system))
        system.getBoolTrackedDeviceProperty=lambda *_:True
        system.getControllerState=lambda *_:(False,S(ulButtonPressed=0))
        self.assertIsNone(hmd.worn_state(vr,system))
        system.getControllerState=lambda *_:(True,S(ulButtonPressed=1<<31))
        self.assertTrue(hmd.worn_state(vr,system))
        system.getControllerState=lambda *_:(True,S(ulButtonPressed=0))
        self.assertFalse(hmd.worn_state(vr,system))

def model_smoke():
    time=1720000000000
    measurements=[]
    for i in range(1200):
        rr=1000 + 35*math.sin(i*.3) + 20*math.sin(i*.11)
        time+=int(rr)
        measurements.append({'timestamp':time,'rr_ms':[rr],'contact':True})
    child=subprocess.run([sys.executable,str(ROOT/'apps/desktop/python/sleep_gru.py')],
                         input=json.dumps({'measurements':measurements})+'\n',text=True,capture_output=True,timeout=120)
    assert child.returncode==0,child.stderr[-3000:]+child.stdout
    outputs=[json.loads(line) for line in child.stdout.splitlines()]
    assert len(outputs)==2,child.stdout
    result=outputs[-1]
    assert result['model']=='ws-gru-mesa' and result['ready'],result
    assert 0<=result['probability']<=1,result
    delay=time-result['evaluated_at']
    assert 260000<=delay<=340000,(delay,result)
    print('PASS: actual official GRU inference, finite probability, delayed epoch, JSON-only stdout:',result)

if __name__=='__main__':
    if '--model' in sys.argv: model_smoke()
    else: unittest.main()
