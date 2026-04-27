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
        body='''<!doctype html><meta charset="utf-8"><title>Tendril Check Radio Task</title>
<style>body{font-family:sans-serif;font-size:30px;margin:40px}.panel{margin-left:760px;margin-top:220px;border:4px solid #ddd;padding:30px;width:700px} label{display:block;margin:25px 0} input{transform:scale(2.0);margin-right:22px} button{font-size:32px;padding:12px;margin-top:20px} #status{color:#064;font-weight:bold;margin-top:30px}</style>
<h1>Tendril Check/Radio Task</h1><p>Choose the checkbox and radio option, then submit.</p>
<form method="POST" action="/submit"><div class="panel"><label><input id="confirm" name="confirm" type="checkbox" value="yes"> Confirm checkbox proof</label><label><input id="choice-green" name="choice" type="radio" value="green"> Green radio proof</label><label><input id="choice-blue" name="choice" type="radio" value="blue"> Blue radio proof</label><button id="submit" type="submit">Submit check radio</button></div></form>
<div id="status">Waiting for check/radio submission.</div>'''
        data=body.encode(); self.send_response(200); self.send_header('Content-Type','text/html; charset=utf-8'); self.send_header('Content-Length',str(len(data))); self.end_headers(); self.wfile.write(data)
    def do_POST(self):
        raw=self.rfile.read(int(self.headers.get('Content-Length','0') or 0)).decode('utf-8','replace')
        fields=parse_qs(raw); confirm=fields.get('confirm',[''])[0]; choice=fields.get('choice',[''])[0]
        result=f'confirm={confirm};choice={choice}'
        (out/'server-checkradio-result.txt').write_text(result)
        body=f'''<!doctype html><meta charset="utf-8"><title>Tendril Check Radio Result</title><style>body{{font-family:sans-serif;font-size:30px;margin:40px}} #status{{color:#064;font-weight:bold}}</style><h1>Check/radio submitted</h1><div id="status">Server wrote choice: {html.escape(result)}</div>'''
        data=body.encode(); self.send_response(200); self.send_header('Content-Type','text/html; charset=utf-8'); self.send_header('Content-Length',str(len(data))); self.end_headers(); self.wfile.write(data)
server=HTTPServer(('127.0.0.1',0), Handler); pathlib.Path(args.port_file).write_text(str(server.server_port)); server.serve_forever()
