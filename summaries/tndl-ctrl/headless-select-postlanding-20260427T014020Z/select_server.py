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
        body='''<!doctype html><meta charset="utf-8"><title>Tendril Select Task</title>
<style>body{font-family:sans-serif;font-size:30px;margin:40px} select{font-size:32px;padding:10px;margin:30px 20px 30px 0} button{font-size:32px;padding:12px} #status{color:#064;font-weight:bold;margin-top:30px}</style>
<h1>Tendril Select Task</h1><p>Choose the Beta option from the dropdown and submit it.</p>
<form method="POST" action="/submit"><label for="choice">Choice</label><select id="choice" name="choice"><option value="">Choose one</option><option value="alpha">Alpha option</option><option value="beta">Beta option</option><option value="gamma">Gamma option</option></select><button id="submit" type="submit">Submit dropdown</button></form>
<div id="status">Waiting for dropdown submission.</div>'''
        data=body.encode(); self.send_response(200); self.send_header('Content-Type','text/html; charset=utf-8'); self.send_header('Content-Length',str(len(data))); self.end_headers(); self.wfile.write(data)
    def do_POST(self):
        raw=self.rfile.read(int(self.headers.get('Content-Length','0') or 0)).decode('utf-8','replace')
        val=parse_qs(raw).get('choice',[''])[0]
        (out/'server-select-choice.txt').write_text(val)
        body=f'''<!doctype html><meta charset="utf-8"><title>Tendril Select Result</title><style>body{{font-family:sans-serif;font-size:30px;margin:40px}} #status{{color:#064;font-weight:bold}}</style><h1>Dropdown submitted</h1><div id="status">Server wrote dropdown choice: {html.escape(val)}</div>'''
        data=body.encode(); self.send_response(200); self.send_header('Content-Type','text/html; charset=utf-8'); self.send_header('Content-Length',str(len(data))); self.end_headers(); self.wfile.write(data)
server=HTTPServer(('127.0.0.1',0), Handler); pathlib.Path(args.port_file).write_text(str(server.server_port)); server.serve_forever()
