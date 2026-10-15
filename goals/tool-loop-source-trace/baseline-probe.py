"""Analyst-only synthetic report/trace gap probe; never owner logs or keys."""
from pathlib import Path
import datetime,hashlib,json,sqlite3,subprocess
BASE=Path(__file__).resolve().parent/'baseline-probe';BIN=Path('target/debug/dendro').resolve()
assert not BASE.exists(),'Preserve original probe evidence'
SRC=BASE/'sources';(SRC/'codex').mkdir(parents=True);DB=BASE/'analytics'/'dendro.duckdb';DB.parent.mkdir()
t0=datetime.datetime(2026,10,15,tzinfo=datetime.timezone.utc)
def at(sec):return (t0+datetime.timedelta(seconds=sec)).isoformat().replace('+00:00','Z')
rows=[{'type':'session_meta','payload':{'id':'synthetic-33-session'}}]
for sec in [0,400,700,800,900,1000,1100]:
 if sec==900:
  rows.append({'type':'response_item','timestamp':at(sec),'payload':{'type':'custom_tool_call','name':'exec','input':'SYNTHETIC_33_BOUNDARY','call_id':'synthetic-boundary'}})
 else:
  rows.append({'type':'event_msg','timestamp':at(sec),'payload':{'type':'item_completed','item':{'type':'CommandExecution','command':['SYNTHETIC_33_CONTENT_CANARY','sk-SYNTHETIC33FAKEONLY'],'exit_code':1}}})
log=SRC/'codex'/'session.jsonl';log.write_text(''.join(json.dumps(x,separators=(',',':'))+'\n' for x in rows))
source=SRC/'openrig.sqlite';c=sqlite3.connect(source)
c.executescript('CREATE TABLE rigs(id TEXT,name TEXT);CREATE TABLE events(seq INTEGER,type TEXT,payload TEXT,created_at TEXT);')
c.execute('INSERT INTO rigs VALUES (?,?)',('probe-rig-id','probe-rig'))
c.execute('INSERT INTO events VALUES (?,?,?,?)',(1,'agent.session_identity',json.dumps({'sessionId':'synthetic-33-session','runtime':'codex','sessionName':'synthetic-seat','rigId':'probe-rig-id'}),'2026-10-15 00:00:00'))
c.commit();c.close()
def sha(p):
 h=hashlib.sha256()
 with p.open('rb') as f:
  for chunk in iter(lambda:f.read(1048576),b''):h.update(chunk)
 return h.hexdigest()
before={str(p.relative_to(BASE)):sha(p) for p in [log,source]}
def run(name,args):
 r=subprocess.run([str(BIN),*args],capture_output=True,text=True)
 (BASE/(name+'.stdout')).write_text(r.stdout);(BASE/(name+'.stderr')).write_text(r.stderr)
 assert r.returncode==0,(name,r.stderr)
 return json.loads(r.stdout)
run('ingest',['run','--db',str(DB),'--root','codex='+str(SRC/'codex'),'--root','openrig='+str(source)])
filters=['--db',str(DB),'--rig','probe-rig','--from',at(0),'--to',at(1800)]
report=run('report',['report','tokens',*filters]);trace=run('trace',['trace','--metric-id','tokens',*filters])
loops=report['payload']['tool_loops'];assert len(loops)==1 and loops[0]['count']==4,loops
summary={'head':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),'binary_sha256':sha(BIN),'report_episodes':loops,'report_coverage':report['payload']['outcome_coverage'],'trace_metric_id':trace['metric_id'],'trace_input_keys':list(trace['inputs']),'trace_has_tool_loops':'tool_loops' in trace['inputs'],'trace_has_loop_trace':'tool_loop_trace' in trace['inputs'],'source_sha256_before':before,'source_sha256_after':{str(p.relative_to(BASE)):sha(p) for p in [log,source]},'canaries_absent_from_public_report_trace':all(v not in json.dumps([report,trace]) for v in ['SYNTHETIC_33_CONTENT_CANARY','sk-SYNTHETIC33FAKEONLY','SYNTHETIC_33_BOUNDARY']),'limits':'synthetic current-entry report/trace gap reproduction; no candidate proof, owner logs/key read, source modification or real-corpus census'}
assert summary['source_sha256_before']==summary['source_sha256_after']
(BASE/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
print(json.dumps(summary,indent=2))
