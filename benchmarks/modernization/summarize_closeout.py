#!/usr/bin/env python3
"""Summarize every recorded closeout outcome without rerunning or filtering cases."""
import argparse,json,statistics,hashlib
from pathlib import Path

def main():
 p=argparse.ArgumentParser();p.add_argument('run',type=Path);a=p.parse_args();root=a.run
 records=json.loads((root/'scorecard.json').read_text());manifest=json.loads((root/'manifest.json').read_text());assert manifest['corpus_sha256']==hashlib.sha256(Path(__file__).with_name('closeout-corpus.json').read_bytes()).hexdigest()
 paired={};totals={v:dict(runs=0,passed=0,foreground_ms=0,wall_seconds=0,requests=0,errors=0,cancelled=0,pending=0,input_tokens=0,output_tokens=0,cache_read_tokens=0,cache_unknown_calls=0,unknown_totals_calls=0) for v in ['A','B']};review=[];runtime_failures=[];reported_pending=[]
 for record in records:
  v=record['version'];name=record['task'];paired.setdefault(name,{})[v]=record;total=totals[v];total['runs']+=1;total['passed']+=record['verified'];total['foreground_ms']+=record['foreground_ms'] or 0;total['wall_seconds']+=record['wall_seconds']
  run=root/f'{name}-{v}';calls=json.loads((run/'result.calls.json').read_text()) if (run/'result.calls.json').exists() else []
  starts=sum(x.get('event')=='request_started' for x in calls);finished=sum(x.get('event')=='request_finished' for x in calls);errors=sum(x.get('event')=='request_failed' for x in calls);cancelled=sum(x.get('event')=='request_cancelled' for x in calls)
  total['requests']+=starts;total['errors']+=errors;total['cancelled']+=cancelled;total['pending']+=starts-finished-errors-cancelled
  for call in calls:
   if call.get('event')!='request_finished':continue
   u=call['response'].get('usage',{})
   for k in ['input_tokens','output_tokens']:total[k]+=u.get(k) or 0
   if u.get('cache_read_tokens') is None:total['cache_unknown_calls']+=1
   else:total['cache_read_tokens']+=u['cache_read_tokens']
   if u.get('totals_reported') is not True:total['unknown_totals_calls']+=1
  result=json.loads((run/'result.json').read_text()) if (run/'result.json').exists() else {}
  if result.get('outcome',{}).get('status')!='returned':runtime_failures.append(dict(task=name,version=v,outcome=result.get('outcome')))
  if result.get('pending_requests_at_report',0):reported_pending.append(dict(task=name,version=v,pending=result['pending_requests_at_report']))
  review.append(f"## {name} / {v}\n\nExternal check: {record['verified']}; process: {record['process_status']}; errors/cancelled/pending: {errors}/{cancelled}/{starts-finished-errors-cancelled}\n\n")
  for msg in result.get('history',[]):
   c=msg.get('content')
   if isinstance(c,list):
    for part in c:
     if part.get('type')=='tool_use':review.append('TOOL '+json.dumps(part,ensure_ascii=False)+'\n\n')
     elif part.get('type')=='tool_result':review.append('RESULT '+json.dumps(part,ensure_ascii=False)+'\n\n')
  review.append('OUTCOME '+json.dumps(result.get('outcome'),ensure_ascii=False)+'\n\n')
 pairs=[]
 for task in manifest['tasks']:
  v=paired.get(task['id'],{})
  if len(v)<2:continue
  aa,bb=v['A'],v['B'];pairs.append(dict(task=task['id'],A=aa['verified'],B=bb['verified'],loss=aa['verified'] and not bb['verified'],gain=bb['verified'] and not aa['verified'],A_ms=aa['foreground_ms'],B_ms=bb['foreground_ms']))
 complete=len(records)==60 and len(pairs)==30
 summary=dict(complete=complete,pairs=pairs,totals=totals,runtime_failures=runtime_failures,reported_pending=reported_pending,losses=sum(x['loss'] for x in pairs),gains=sum(x['gain'] for x in pairs),protected_corruptions=[f"{x['task']}-{x['version']}" for x in records if not x['protected_unchanged']],process_failures=[f"{x['task']}-{x['version']}" for x in records if x['process_status']!=0],observed_artifact_gate_pass=complete and all(not x['loss'] for x in pairs) and totals['B']['passed']>=totals['A']['passed'] and all(x['protected_unchanged'] for x in records if x['version']=='B'),release_authorized_by_this_script=False)
 (root/'summary.json').write_text(json.dumps(summary,indent=2)+'\n');(root/'claim-review.md').write_text(''.join(review))
 print(json.dumps({k:v for k,v in summary.items() if k!='pairs'},indent=2))
if __name__=='__main__':main()
