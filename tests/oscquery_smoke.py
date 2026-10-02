"""OSCQuery integration with a local VRChat-shaped mDNS/HTTP/UDP test peer."""
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
healthy = threading.Event()
healthy.set()
stop = threading.Event()
queries = []
class Query(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        queries.append(self.path)
        self.send_response(200 if healthy.is_set() else 503)
        self.send_header('Content-Type', 'application/json')
        self.end_headers()
        self.wfile.write(b'{"FULL_PATH":"/","ACCESS":0,"CONTENTS":{}}')
    def log_message(self, *args):
        pass

def name(value):
    return b''.join(bytes([len(part)]) + part.encode() for part in value.split('.')) + b'\0'
def record(owner, kind, data):
    return name(owner) + struct.pack('!HHIH', kind, 1, 120, len(data)) + data

def announce(kind, port):
    service = '_oscjson._tcp.local' if kind == 'query' else '_osc._udp.local'
    instance = 'VRChat-Client-NemuriBeatTest.' + service
    records = [record(service, 12, name(instance)),
               record(instance, 33, struct.pack('!HHH', 0, 0, port) + name(instance)),
               record(instance, 1, socket.inet_aton('127.0.0.1'))]
    return struct.pack('!HHHHHH', 0, 0x8400, 0, 1, 0, 2) + b''.join(records)

with tempfile.TemporaryDirectory(prefix='nemuri-query-') as tmp:
    root = Path(tmp)
    personal = root / 'Demo/PersonalData/nemuribeat.sqlite'
    personal.parent.mkdir(parents=True)
    udp = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    udp.bind(('127.0.0.1', 0))
    udp.settimeout(12)
    decoy = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    decoy.bind(('127.0.0.1', 0))
    decoy.settimeout(.1)
    query = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Query)
    threading.Thread(target=query.serve_forever, daemon=True).start()
    db = sqlite3.connect(personal)
    db.executescript((REPO / 'migrations/001_initial.sql').read_text())
    db.execute("INSERT INTO settings VALUES ('app',?,0)", [json.dumps({
        'osc_enabled': True, 'oscquery_enabled': True,
        'osc_port': decoy.getsockname()[1]})])
    db.commit()
    process = subprocess.Popen([BINARY, '--demo', '--headless', '--seconds', '24', '--data-dir', str(root)],
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    def broadcast():
        multicast = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        multicast.setsockopt(socket.IPPROTO_IP, socket.IP_MULTICAST_LOOP, 1)
        while not stop.wait(.4):
            for interface in [None, '127.0.0.1']:
                if interface:
                    multicast.setsockopt(socket.IPPROTO_IP, socket.IP_MULTICAST_IF, socket.inet_aton(interface))
                else:
                    multicast.setsockopt(socket.IPPROTO_IP, socket.IP_MULTICAST_IF, socket.inet_aton('0.0.0.0'))
                for kind, port in [('osc', udp.getsockname()[1]), ('query', query.server_port)]:
                    multicast.sendto(announce(kind, port), ('224.0.0.251', 5353))
        multicast.close()
    broadcaster = threading.Thread(target=broadcast, daemon=True)
    broadcaster.start()
    try:
        packet = udp.recv(4096)
        assert packet.startswith(b'/avatar/parameters/VRCOSC/'), packet
        assert queries, 'Discovery must verify the OSCQuery HTTP endpoint'
        try:
            decoy.recv(4096)
            raise AssertionError('Auto mode must not use the configured manual destination')
        except socket.timeout:
            pass
        healthy.clear()
        time.sleep(7.2)
        udp.setblocking(False)
        while True:
            try: udp.recv(4096)
            except BlockingIOError: break
        udp.settimeout(1)
        try:
            udp.recv(4096)
            raise AssertionError('Sending must stop after OSCQuery endpoint becomes unavailable')
        except socket.timeout:
            pass
        healthy.set()
        udp.settimeout(7)
        assert udp.recv(4096).startswith(b'/avatar/parameters/VRCOSC/'), 'Sending should recover'
        out, err = process.communicate(timeout=26)
        assert process.returncode == 0, err
        assert 'Stopped cleanly.' in out
        assert db.execute('SELECT COUNT(*) FROM raw_measurements').fetchone()[0] >= 20
        print('PASS: mDNS discovery, OSCQuery HTTP verification, dynamic UDP destination, outage stop, recovery, sensor/DB continuity')
    finally:
        stop.set()
        if process.poll() is None:
            process.terminate()
            process.communicate(timeout=5)
        query.shutdown()
        query.server_close()
        udp.close()
        decoy.close()
        db.close()
