"""OyasumiVR-shaped local peer: mDNS, verified OSCQuery, Bool OSC and restart."""
import http.server
import json
from pathlib import Path
import socket
import sqlite3
import struct
import subprocess
import sys
import tempfile
import threading
import time

BINARY = str(Path(sys.argv[1]).resolve())
REPO = Path(__file__).resolve().parents[1]
stop = threading.Event()
healthy = threading.Event()
healthy.set()
state = {'port': 0, 'asleep': True}
queries = []

class Query(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        queries.append(self.path)
        self.send_response(200 if healthy.is_set() else 503)
        self.send_header('Content-Type', 'application/json')
        self.end_headers()
        value = {'NAME': 'OyasumiVR', 'OSC_TRANSPORT': 'UDP',
                 'OSC_PORT': state['port'], 'OSC_IP': '127.0.0.1'} if self.path == '/?HOST_INFO' else {
                     'FULL_PATH': '/OyasumiVR/SleepMode', 'ACCESS': 3,
                     'VALUE_TYPE': 'F', 'VALUE': [state['asleep']]}
        self.wfile.write(json.dumps(value).encode())
    def log_message(self, *args):
        pass

def name(value):
    return b''.join(bytes([len(p)]) + p.encode() for p in value.split('.')) + b'\0'

def record(owner, kind, data):
    return name(owner) + struct.pack('!HHIH', kind, 1, 120, len(data)) + data

def announce(instance, port):
    service = '_oscjson._tcp.local'
    instance += '.' + service
    records = [record(service, 12, name(instance)),
               record(instance, 33, struct.pack('!HHH', 0, 0, port) + name(instance)),
               record(instance, 1, socket.inet_aton('127.0.0.1'))]
    return struct.pack('!HHHHHH', 0, 0x8400, 0, 1, 0, 2) + b''.join(records)

def udp_peer():
    peer = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    peer.bind(('127.0.0.1', 0))
    return peer

def receive_awake(peer, timeout):
    peer.settimeout(timeout)
    # OSC has a Bool F typetag and no numeric argument payload.
    packet = peer.recv(1024)
    address = b'/OyasumiVR/SleepMode\0'
    address += b'\0' * (-len(address) % 4)
    assert packet == address + b',F\0\0', packet
    state['asleep'] = False

with tempfile.TemporaryDirectory(prefix='nemuri-oyasumi-') as tmp:
    root = Path(tmp)
    personal = root / 'Demo/PersonalData/nemuribeat.sqlite'
    personal.parent.mkdir(parents=True)
    first, second, decoy = udp_peer(), udp_peer(), udp_peer()
    state['port'] = first.getsockname()[1]
    query = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Query)
    threading.Thread(target=query.serve_forever, daemon=True).start()
    db = sqlite3.connect(personal)
    db.executescript((REPO / 'migrations/001_initial.sql').read_text())
    with socket.socket() as reservation:
        reservation.bind(('127.0.0.1', 0))
        api_port = reservation.getsockname()[1]
    db.execute("INSERT INTO settings VALUES ('app',?,0)", [json.dumps({
        'oyasumi_enabled': True, 'oyasumi_query_port': 0, 'oyasumi_allow_hr_only': True,
        'api_port': api_port, 'osc_enabled': False, 'osc_port': decoy.getsockname()[1]})])
    db.commit()
    process = subprocess.Popen([BINARY, '--demo-hr-only', '--headless', '--seconds', '26', '--data-dir', str(root)],
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    def broadcast():
        multicast = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        multicast.setsockopt(socket.IPPROTO_IP, socket.IP_MULTICAST_LOOP, 1)
        while not stop.wait(.4):
            for interface in ['0.0.0.0', '127.0.0.1']:
                multicast.setsockopt(socket.IPPROTO_IP, socket.IP_MULTICAST_IF, socket.inet_aton(interface))
                multicast.sendto(announce('OyasumiVR', query.server_port), ('224.0.0.251', 5353))
        multicast.close()
    threading.Thread(target=broadcast, daemon=True).start()
    try:
        receive_awake(first, 12)
        assert '/?HOST_INFO' in queries and '/OyasumiVR/SleepMode' in queries
        first.settimeout(5.2)
        try:
            first.recv(1024)
            raise AssertionError('Readback-matched state must not be resent')
        except socket.timeout:
            pass
        healthy.clear()
        state['asleep'] = True
        time.sleep(5.2)
        first.settimeout(.3)
        try:
            first.recv(1024)
            raise AssertionError('Unavailable HTTP endpoint must stop sends')
        except socket.timeout:
            pass
        state['port'] = second.getsockname()[1]
        healthy.set()
        receive_awake(second, 7)
        first.settimeout(.3)
        decoy.settimeout(.3)
        for peer in [first, decoy]:
            try:
                peer.recv(1024)
                raise AssertionError('Must not use stale port or VRChat manual port')
            except socket.timeout:
                pass
        out, err = process.communicate(timeout=30)
        assert process.returncode == 0, err
        assert 'Stopped cleanly.' in out
        assert db.execute('SELECT COUNT(*) FROM raw_measurements').fetchone()[0] >= 25
        print('PASS: OyasumiVR mDNS, HOST_INFO/Bool readback, no repeated writes, outage hold, dynamic OSC port recovery, independent VRChat toggle, DB continuity')
    finally:
        stop.set()
        if process.poll() is None:
            process.terminate()
            process.communicate(timeout=5)
        query.shutdown()
        query.server_close()
        for peer in [first, second, decoy]:
            peer.close()
        db.close()
