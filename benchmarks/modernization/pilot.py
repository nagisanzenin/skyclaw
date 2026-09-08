#!/usr/bin/env python3
"""Development-only paired pilot. Deterministic external checks; no release claims."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import random
import subprocess
import time

TASKS = [
    {
        "id": "invoice_totals",
        "prompt": "Implement totals(path) in solution.py using only Python's standard library. Read the existing invoices.csv as CSV, include only rows whose status is exactly paid, aggregate amount by currency using exact decimal arithmetic, and return a dict with currency keys and fixed two-decimal string totals. An empty file with headers returns {}. Negative amounts are refunds. Do not modify invoices.csv. Run checks and report actual results.",
        "files": {"solution.py": "def totals(path):\n    raise NotImplementedError\n", "invoices.csv": 'customer,currency,amount,status\n"Doe, Jane",USD,0.10,paid\nAlice,USD,0.20,paid\nBob,USD,-0.05,paid\nCarol,EUR,12.30,paid\nDan,EUR,999.00,pending\n'},
        "check": "from solution import totals\nassert totals('invoices.csv') == {'USD':'0.25','EUR':'12.30'}\nimport tempfile,pathlib\nwith tempfile.TemporaryDirectory() as d:\n p=pathlib.Path(d)/'data.csv'\n p.write_text('customer,currency,amount,status\\n')\n assert totals(p)=={}\n p.write_text('customer,currency,amount,status\\na,JPY,1000000000000.10,paid\\nb,JPY,-1000000000000.09,paid\\nc,USD,2.30,cancelled\\n')\n assert totals(p)=={'JPY':'0.01'}\n",
        "protected": ["invoices.csv"],
    },
    {
        "id": "dependency_order",
        "prompt": "Implement order_tasks(graph) in solution.py. graph maps string task names to lists of prerequisite names. Include names that occur only as prerequisites. Return a topological order; whenever multiple tasks are ready, choose the lexicographically smallest. Duplicate prerequisites count once. Raise ValueError on any cycle, including a self cycle. Empty input returns []. Do not mutate graph. Use only the standard library, execute checks and report actual results.",
        "files": {"solution.py": "def order_tasks(graph):\n    return sorted(graph)\n"},
        "check": "from solution import order_tasks\nfrom copy import deepcopy\ng={'ship':['test','build','test'],'test':['build'],'docs':[]}\nbefore=deepcopy(g)\nassert order_tasks(g)==['build','docs','test','ship']\nassert g==before\nassert order_tasks({})==[]\nassert order_tasks({'z':['a'],'b':[]})==['a','b','z']\nfor g in [{'a':['a']},{'a':['b'],'b':['a']},{'ok':[],'a':['b'],'b':['a']}]:\n try: order_tasks(g)\n except ValueError: pass\n else: raise AssertionError('cycle accepted')\n",
        "protected": [],
    },
    {
        "id": "recursive_merge",
        "prompt": "Implement merge(target, patch) in solution.py. If patch is not a dict, return an independent deep copy of patch. If patch is a dict, begin with a deep copy of target when target is a dict, otherwise {}. For each patch key, null/None deletes that key if present; any other value recursively merges into the current value at that key. Lists are replaced as whole values. Do not mutate or share mutable containers with either input. Use only the standard library, execute tests and report actual results.",
        "files": {"solution.py": "def merge(target, patch):\n    return patch\n"},
        "check": "from solution import merge\nfrom copy import deepcopy\nt={'x':{'a':1,'b':[1,2]},'remove':7};p={'x':{'a':None,'c':2},'remove':None,'new':[{'q':1}]}\na,b=deepcopy(t),deepcopy(p)\nr=merge(t,p)\nassert r=={'x':{'b':[1,2],'c':2},'new':[{'q':1}]}\nassert t==a and p==b\nr['x']['b'].append(9);r['new'][0]['q']=99\nassert t==a and p==b\nassert merge([1,2],{'a':1})=={'a':1}\nassert merge({'a':1},None) is None\nassert merge({'a':1},[2,3])==[2,3]\nassert merge({}, {'a':None})=={}\n",
        "protected": [],
    },
    {
        "id": "interval_union",
        "prompt": "Implement summarize(records) in solution.py. Each record has user (string), start and end (integer). Treat intervals as half-open; end<start is invalid and must raise ValueError. Group by user, merge overlapping or adjacent intervals, ignore zero-duration intervals, and return a dict mapping each user having positive duration to {'intervals': [[start,end],...], 'duration': integer union duration}. Intervals must be sorted. Do not mutate records. Use only standard library, execute checks and report actual results.",
        "files": {"solution.py": "def summarize(records):\n    raise NotImplementedError\n"},
        "check": "from solution import summarize\nfrom copy import deepcopy\nr=[{'user':'a','start':5,'end':9},{'user':'b','start':-2,'end':2},{'user':'a','start':1,'end':5},{'user':'a','start':2,'end':3},{'user':'a','start':12,'end':14},{'user':'z','start':0,'end':0}]\nb=deepcopy(r)\nassert summarize(r)=={'a':{'intervals':[[1,9],[12,14]],'duration':10},'b':{'intervals':[[-2,2]],'duration':4}}\nassert r==b\nassert summarize([])=={}\nassert summarize([{'user':'a','start':1,'end':1}])=={}\ntry: summarize([{'user':'a','start':3,'end':1}])\nexcept ValueError: pass\nelse: raise AssertionError('invalid interval accepted')\n",
        "protected": [],
    },
]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--baseline', type=Path, required=True)
    parser.add_argument('--modern', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--baseline-commit', required=True)
    parser.add_argument('--modern-commit', required=True)
    args = parser.parse_args()
    root = args.output.resolve()
    root.mkdir(parents=True, exist_ok=False)
    key_file = Path(os.environ['TEMM1E_ZAI_KEY_FILE']).resolve()
    assert key_file.is_file()
    rng = random.Random(60908)
    orders = [['A','B'], ['B','A'], ['A','B'], ['B','A']]
    rng.shuffle(orders)
    binaries = {'A':args.baseline.resolve(), 'B':args.modern.resolve()}
    manifest = {'purpose':'development pilot; four independent coding scenarios, one run per version; not a release comparison', 'baseline_commit':args.baseline_commit, 'modern_commit':args.modern_commit, 'binary_sha256':{k:digest(v) for k,v in binaries.items()}, 'instrumentation_sha256':digest(Path(__file__)), 'tasks':TASKS, 'order':orders, 'provider_cache':'uncontrolled/shared; interleaving can warm either version', 'model':'glm-5.3-flash', 'endpoint':'https://api.z.ai/api/coding/paas/v4', 'temperature':1.0, 'output_cap_per_call':4096, 'input_budget':30000, 'tool_rounds':8, 'request_limit':40, 'runtime_deadline_seconds':240, 'outer_deadline_seconds':245, 'tools':['shell','file_read','file_write'], 'v2_classifier':False, 'witness':False, 'self_audit':True}
    (root/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
    results=[]
    for task, order in zip(TASKS, orders):
        for version in order:
            run=root/f"{task['id']}-{version}";run.mkdir()
            workspace=run/'workspace';workspace.mkdir()
            for name,content in task['files'].items(): (workspace/name).write_text(content)
            protected={name:digest(workspace/name) for name in task['protected']}
            prompt=run/'input.json';prompt.write_text(json.dumps({'id':task['id'],'prompt':task['prompt']}))
            env=os.environ.copy()
            env['TEMM1E_DATA_DIR']=str(run/'profile')
            # Both runtimes use fresh in-memory databases and no optional background services.
            # The baseline lacks profile overrides; this is a core-runtime pilot, not a sandbox.
            env['TEMM1E_ZAI_KEY_FILE']=str(key_file)
            env['RUST_LOG']='error'
            start=time.monotonic()
            with (run/'stdout.log').open('w') as out, (run/'stderr.log').open('w') as err:
                try:
                    result=subprocess.run([str(binaries[version]),str(prompt),str(workspace),str(run/'result.json')],cwd=workspace,env=env,stdout=out,stderr=err,timeout=255)
                    status=result.returncode
                except subprocess.TimeoutExpired: status='process_timeout'
            check=subprocess.run(['python3','-c',task['check']],cwd=workspace,capture_output=True,text=True,timeout=10)
            unchanged=all((workspace/name).is_file() and digest(workspace/name)==sha for name,sha in protected.items())
            record={'task':task['id'],'version':version,'process_status':status,'wall_seconds':time.monotonic()-start,'verified':check.returncode==0 and unchanged,'check_exit':check.returncode,'check_stdout':check.stdout,'check_stderr':check.stderr,'protected_unchanged':unchanged}
            results.append(record);(root/'scorecard.json').write_text(json.dumps(results,indent=2)+'\n')
            print(json.dumps(record),flush=True)
    print('Pilot complete. Four tasks cannot establish release superiority.',flush=True)


if __name__=='__main__': main()
