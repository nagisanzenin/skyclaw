#!/usr/bin/env python3
"""Single frozen clarification pair; retains original ambiguous result."""
import argparse,hashlib,json,os,random,subprocess,time
from pathlib import Path

def sha(path): return hashlib.sha256(path.read_bytes()).hexdigest()
def main():
 p=argparse.ArgumentParser()
 for name in ['baseline','modern','output']: p.add_argument('--'+name,type=Path,required=True)
 for name in ['baseline-commit','modern-commit']: p.add_argument('--'+name,required=True)
 args=p.parse_args();corpus=Path(__file__).with_name('closeout-clarification.json');tasks=[json.loads(corpus.read_text())['corrected_task']];assert len(tasks)==1
 root=args.output.resolve();root.mkdir(parents=True,exist_ok=False)
 key=Path(os.environ['TEMM1E_ZAI_KEY_FILE']).resolve();assert key.is_file()
 bins={'A':args.baseline.resolve(),'B':args.modern.resolve()}
 orders=[['B','A']]
 manifest=dict(purpose='one contract clarification pair; original ambiguous30-pair evidence retained; not a new independent scenario',baseline_commit=args.baseline_commit,modern_commit=args.modern_commit,binary_sha256={k:sha(v) for k,v in bins.items()},instrumentation_sha256=sha(Path(__file__)),corpus_sha256=sha(corpus),tasks=tasks,order=orders,model='glm-5.3-flash',endpoint='https://api.z.ai/api/coding/paas/v4',temperature=1,output_cap=4096,input_budget=30000,tool_rounds=8,request_limit=40,runtime_deadline=240,outer_deadline=245,post_turn_drain=125,process_deadline=380,provider_cache='uncontrolled shared account; balanced interleaving',resource_mode='equal core harness; same Rust instrumentation as pilot02; classifier/Witness disabled')
 (root/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n');results=[]
 for task,order in zip(tasks,orders):
  for version in order:
   run=root/f"{task['id']}-{version}";run.mkdir();workspace=run/'workspace';workspace.mkdir()
   for name,content in task['files'].items():
    f=workspace/name;f.parent.mkdir(parents=True,exist_ok=True);f.write_text(content)
   protected={name:sha(workspace/name) for name in task['protected']}
   prompt=run/'input.json';prompt.write_text(json.dumps({'id':task['id'],'prompt':task['prompt']}))
   env=os.environ.copy();env.update(TEMM1E_DATA_DIR=str(run/'profile'),TEMM1E_ZAI_KEY_FILE=str(key),RUST_LOG='error')
   start=time.monotonic()
   with (run/'stdout.log').open('w') as out,(run/'stderr.log').open('w') as err:
    try: status=subprocess.run([str(bins[version]),str(prompt),str(workspace),str(run/'result.json')],cwd=workspace,env=env,stdout=out,stderr=err,timeout=380).returncode
    except subprocess.TimeoutExpired: status='process_timeout'
   runtime=json.loads((run/'result.json').read_text()) if (run/'result.json').exists() else {}
   try:
    check=subprocess.run(['python3','-c',task['check']],cwd=workspace,capture_output=True,text=True,timeout=10)
    checkcode,stdout,stderr=check.returncode,check.stdout,check.stderr
   except subprocess.TimeoutExpired: checkcode,stdout,stderr='timeout','','external check exceeded10seconds'
   unchanged=all((workspace/n).is_file() and sha(workspace/n)==h for n,h in protected.items())
   record=dict(task=task['id'],version=version,process_status=status,wall_seconds=time.monotonic()-start,foreground_ms=runtime.get('foreground_ms'),verified=checkcode==0 and unchanged,check_exit=checkcode,check_stdout=stdout,check_stderr=stderr,protected_unchanged=unchanged)
   results.append(record);(root/'scorecard.json').write_text(json.dumps(results,indent=2)+'\n');print(json.dumps(record),flush=True)
 print('Complete; inspect all outcomes, pending usage and unsupported claims before release decision.',flush=True)
if __name__=='__main__': main()
