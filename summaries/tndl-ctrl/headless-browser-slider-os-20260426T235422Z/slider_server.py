#!/usr/bin/env python3
from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.parse import parse_qs
import argparse, html, pathlib
parser=argparse.ArgumentParser(); parser.add_argument('--port-file', required=True); parser.add_argument('--out-dir', required=True); args=parser.parse_args()
out=pathlib.Path(args.out_dir); out.mkdir(parents=True, exist_ok=True)
class Handler(BaseHTTPRequestHandler):
    def log_message(self, fmt, *params):
        with (out/'server.log').open('a') as f: f.write(fmt % params + '\n')
    def do_GET(self):
        body='''<!doctype html><meta charset="utf-8"><title>Tendril Slider Task</title>
<style>body{font-family:sans-serif;font-size:30px;margin:40px} input[type=range]{width:1000px;height:80px;display:block;margin:40px 0} button{font-size:32px;padding:12px} #status{color:#064;font-weight:bold;margin-top:30px}</style>
<h1>Tendril Slider Task</h1><p>Drag the slider to a high value and submit it.</p>
<form method="POST" action="/submit"><label for="level">Level: <output id="out">0</output></label><input id="level" name="level" type="range" min="0" max="100" value="0" oninput="out.value=this.value"><button id="submit" type="submit">Submit slider value</button></form>
<div id="status">Waiting for slider submission.</div>'''
        data=body.encode(); self.send_response(200); self.send_header('Content-Type','text/html; charset=utf-8'); self.send_header('Content-Length',str(len(data))); self.end_headers(); self.wfile.write(data)
    def do_POST(self):
        raw=self.rfile.read(int(self.headers.get('Content-Length','0') or 0)).decode('utf-8','replace')
        val=parse_qs(raw).get('level',[''])[0]
        (out/'server-slider-value.txt').write_text(val)
        body=f'''<!doctype html><meta charset="utf-8"><title>Tendril Slider Result</title><style>body{{font-family:sans-serif;font-size:30px;margin:40px}} #status{{color:#064;font-weight:bold}}</style><h1>Slider submitted</h1><div id="status">Server wrote slider value: {html.escape(val)}</div>'''
        data=body.encode(); self.send_response(200); self.send_header('Content-Type','text/html; charset=utf-8'); self.send_header('Content-Length',str(len(data))); self.end_headers(); self.wfile.write(data)
server=HTTPServer(('127.0.0.1',0), Handler); pathlib.Path(args.port_file).write_text(str(server.server_port)); server.serve_forever()
