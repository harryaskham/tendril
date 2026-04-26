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
        body='''<!doctype html><meta charset="utf-8"><title>Tendril Keyboard Form Task</title>
<style>body{font-family:sans-serif;font-size:28px;margin:40px} input{display:block;font-size:30px;width:850px;padding:10px;margin:18px 0} button{font-size:30px;padding:10px;margin-top:8px} #status{margin-top:30px;color:#064;font-weight:bold}</style>
<h1>Tendril Keyboard Form Task</h1><p>Use keyboard Tab and Enter to submit two fields.</p>
<form method="POST" action="/submit"><label>First proof<input id="first" name="first" autofocus></label><label>Second proof<input id="second" name="second"></label><button id="submit" type="submit">Submit keyboard form</button></form>
<div id="status">Waiting for keyboard-only submission.</div>'''
        data=body.encode(); self.send_response(200); self.send_header('Content-Type','text/html; charset=utf-8'); self.send_header('Content-Length',str(len(data))); self.end_headers(); self.wfile.write(data)
    def do_POST(self):
        raw=self.rfile.read(int(self.headers.get('Content-Length','0') or 0)).decode('utf-8','replace')
        fields=parse_qs(raw); first=fields.get('first',[''])[0]; second=fields.get('second',[''])[0]
        msg=f'{first}|{second}'; (out/'server-keyform-submission.txt').write_text(msg)
        body=f'''<!doctype html><meta charset="utf-8"><title>Tendril Keyboard Form Result</title><style>body{{font-family:sans-serif;font-size:30px;margin:40px}} #status{{color:#064;font-weight:bold}}</style><h1>Keyboard form submitted</h1><div id="status">Server wrote OS file: {html.escape(msg)}</div>'''
        data=body.encode(); self.send_response(200); self.send_header('Content-Type','text/html; charset=utf-8'); self.send_header('Content-Length',str(len(data))); self.end_headers(); self.wfile.write(data)
server=HTTPServer(('127.0.0.1',0), Handler); pathlib.Path(args.port_file).write_text(str(server.server_port)); server.serve_forever()
