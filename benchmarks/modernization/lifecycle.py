"""Explicit local CLI/server lifecycle smoke; no live provider or external channels."""
from pathlib import Path
import os, subprocess, socket, time, signal, urllib.request, json, argparse
parser=argparse.ArgumentParser()
parser.add_argument('--binary',type=Path,required=True)
parser.add_argument('--output',type=Path,required=True)
args=parser.parse_args()
root=args.output.resolve();root.mkdir(parents=True,exist_ok=False)
binary=args.binary.resolve()
env={k:v for k,v in os.environ.items() if not any(x in k.upper() for x in ['TOKEN','API_KEY','SECRET','TEMM1E_'])}
env['TEMM1E_DATA_DIR']=str(root/'data');env['RUST_LOG']='error'
with socket.socket() as s:
 s.bind(('127.0.0.1',0));port=s.getsockname()[1]
config=root/'config.toml';config.write_text(f'''[gateway]
host = "127.0.0.1"
port = {port}
[provider]
name = "openai-compatible"
api_key = "offline-fixture"
model = "fixture-model"
base_url = "http://127.0.0.1:9"
[heartbeat]
enabled = false
[perpetuum]
enabled = false
[consciousness]
enabled = false
[witness]
enabled = false
[hive]
enabled = false
''')
report={}
for command,input_text in [('chat','/quit\n'),('chat','')]:
 start=time.monotonic()
 r=subprocess.run([str(binary),'--config',str(config),command],input=input_text,text=True,capture_output=True,cwd=root,env=env,timeout=20)
 label='quit' if input_text else 'eof'
 (root/f'{label}.stdout.log').write_text(r.stdout);(root/f'{label}.stderr.log').write_text(r.stderr)
 report[label]={'exit':r.returncode,'seconds':time.monotonic()-start,'ended':'TEMM1E chat ended.' in r.stdout}
 assert r.returncode==0 and report[label]['ended'],report[label]
with (root/'server.stdout.log').open('w') as out, (root/'server.stderr.log').open('w') as err:
 p=subprocess.Popen([str(binary),'--config',str(config),'start'],cwd=root,env=env,stdout=out,stderr=err)
 try:
  deadline=time.monotonic()+20
  while time.monotonic()<deadline:
   if p.poll() is not None: raise AssertionError(f'server exited {p.returncode}')
   try:
    with urllib.request.urlopen(f'http://127.0.0.1:{port}/health',timeout=.5) as response:
     if response.status==200: break
   except Exception: time.sleep(.1)
  else: raise AssertionError('health readiness timeout')
  time.sleep(.2)
  start=time.monotonic();p.send_signal(signal.SIGTERM);code=p.wait(timeout=15)
  report['sigterm']={'exit':code,'seconds':time.monotonic()-start}
  assert code==0,report['sigterm']
 finally:
  if p.poll() is None: p.kill();p.wait()
(root/'evidence.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))
