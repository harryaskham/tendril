#!/usr/bin/env python3
from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.parse import parse_qs
import argparse, html, pathlib
parser=argparse.ArgumentParser()
parser.add_argument('--port-file', required=True)
parser.add_argument('--out-dir', required=True)
args=parser.parse_args()
out=pathlib.Path(args.out_dir); out.mkdir(parents=True, exist_ok=True)
class Handler(BaseHTTPRequestHandler):
    def log_message(self, fmt, *params):
        with (out/'server.log').open('a') as f: f.write(fmt % params + '\n')
    def do_GET(self):
        body='''<!doctype html><meta charset="utf-8"><title>Tendril WebPost Task</title>
<style>body{font-family:sans-serif;font-size:28px;margin:40px} input{font-size:30px;width:900px;padding:10px} button{font-size:30px;padding:10px;margin-left:12px} #status{margin-top:30px;color:#064;font-weight:bold}</style>
<h1>Tendril WebPost Task</h1><p>Submit browser text to a local OS server file.</p>
<form method="POST" action="/submit"><input id="message" name="message" autofocus value=""><button id="submit" type="submit">Write OS file</button></form>
<div id="status">Waiting for browser submission.</div>'''
        data=body.encode(); self.send_response(200); self.send_header('Content-Type','text/html; charset=utf-8'); self.send_header('Content-Length',str(len(data))); self.end_headers(); self.wfile.write(data)
    def do_POST(self):
        n=int(self.headers.get('Content-Length','0') or 0); raw=self.rfile.read(n).decode('utf-8','replace')
        fields=parse_qs(raw); msg=fields.get('message',[''])[0]
        (out/'server-submission.txt').write_text(msg)
        body=f'''<!doctype html><meta charset="utf-8"><title>Tendril WebPost Result</title><style>body{{font-family:sans-serif;font-size:30px;margin:40px}} #status{{color:#064;font-weight:bold}}</style><h1>Submitted</h1><div id="status">Server wrote OS file: {html.escape(msg)}</div>'''
        data=body.encode(); self.send_response(200); self.send_header('Content-Type','text/html; charset=utf-8'); self.send_header('Content-Length',str(len(data))); self.end_headers(); self.wfile.write(data)
server=HTTPServer(('127.0.0.1',0), Handler)
pathlib.Path(args.port_file).write_text(str(server.server_port))
server.serve_forever()
