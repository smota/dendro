"""Synthetic analyst source-pruning trace study; no owner paths or key reads."""
from pathlib import Path
import datetime,hashlib,json,os,sqlite3,subprocess
BASE=Path(__file__).resolve().parent/'baseline-probe';BIN=Path('target/debug/dendro').resolve()
assert not BASE.exists(),'Preserve original probe evidence'
SRC=BASE/'sources';SRC.mkdir(parents=True);DB=BASE/'analytics'/'dendro.duckdb';DB.parent.mkdir()
t0=datetime.datetime(2026,10,15,tzinfo=datetime.timezone.utc)
def at(sec):return (t0+datetime.timedelta(seconds=sec)).isoformat().replace('+00:00','Z')
claude=SRC/'claude'/'session.jsonl';pi=SRC/'pi'/'session.jsonl';codex=SRC/'codex'/'session.jsonl'
def write(p,rows):
 p.parent.mkdir(parents=True,exist_ok=True);p.write_text(''.join(json.dumps(x,separators=(',',':'))+'\n' for x in rows))
write(claude,[{'type':'assistant','sessionId':'synthetic-32-claude','timestamp':at(1),'message':{'id':'c1','usage':{'input_tokens':3,'output_tokens':0,'cache_read_input_tokens':0,'cache_creation_input_tokens':0}}}])
write(pi,[{'type':'session','version':3,'id':'synthetic-32-pi','timestamp':at(0)},{'type':'message','id':'p1','timestamp':at(1),'message':{'role':'assistant','usage':{'input':5,'output':0,'cacheRead':0,'cacheWrite':0}}}])
write(codex,[{'type':'session_meta','timestamp':at(0),'payload':{'id':'synthetic-32-codex'}},{'type':'event_msg','timestamp':at(1),'payload':{'type':'token_count','info':{'total_token_usage':{'input_tokens':7,'cached_input_tokens':0,'cache_write_input_tokens':0,'output_tokens':0,'reasoning_output_tokens':0}}}},*[{'type':'event_msg','timestamp':at(sec),'payload':{'type':'item_completed','item':{'type':'CommandExecution','command':['SYNTHETIC_32_COMMAND_CANARY'],'exit_code':1}}} for sec in [2,3,4]]])
openrig=SRC/'openrig.sqlite';c=sqlite3.connect(openrig)
c.executescript('CREATE TABLE rigs(id TEXT,name TEXT);CREATE TABLE events(seq INTEGER,type TEXT,payload TEXT,created_at TEXT);')
c.execute('INSERT INTO rigs VALUES (?,?)',('probe-id','probe-rig'))
for i,(runtime,native) in enumerate([('claude-code','synthetic-32-claude'),('pi','synthetic-32-pi'),('codex','synthetic-32-codex')],1):
 c.execute('INSERT INTO events VALUES (?,?,?,?)',(i,'agent.session_identity',json.dumps({'sessionId':native,'runtime':runtime,'sessionName':'seat-'+str(i),'rigId':'probe-id'}),'2026-10-15 00:00:00'))
c.commit();c.close()
def sha(p):
 h=hashlib.sha256()
 with p.open('rb') as f:
  for b in iter(lambda:f.read(1048576),b''):h.update(b)
 return h.hexdigest()
def run(label,args):
 r=subprocess.run([str(BIN),*args],capture_output=True,text=True)
 (BASE/(label+'.stdout')).write_text(r.stdout);(BASE/(label+'.stderr')).write_text(r.stderr)
 assert r.returncode==0,(label,r.stderr)
 return json.loads(r.stdout)
args=['run','--db',str(DB)]
for h in ['claude','pi','codex']:args+=['--root',h+'='+str(SRC/h)]
args+=['--root','openrig='+str(openrig)];run('ingest',args)
filters=['--db',str(DB),'--rig','probe-rig','--from',at(0),'--to',at(600)]
initial_report=run('initial-report',['report','tokens',*filters]);initial_trace=run('initial-trace',['trace','--metric-id','tokens',*filters]);db_before=sha(DB)
assert initial_report['payload']['tokens']==15,initial_report['payload']['tokens']
assert initial_report['payload']['tool_loops'][0]['count']==3
checks=[]
def compare(name,change):
 change()
 report=run(name+'-report',['report','tokens',*filters]);trace=run(name+'-trace',['trace','--metric-id','tokens',*filters])
 states=[{'source_id':x['source_id'],'generation':x['generation'],'current_state':x['current_state']} for x in trace['inputs']['tool_loop_trace']['sources']]
 row={'case':name,'report_exactly_equal':report==initial_report,'trace_exactly_equal':trace==initial_trace,'tokens':report['payload']['tokens'],'loop_count':report['payload']['tool_loops'][0]['count'],'source_states':states,'analytics_bytes_unchanged':sha(DB)==db_before}
 assert row['report_exactly_equal'] and row['trace_exactly_equal'] and row['analytics_bytes_unchanged']
 checks.append(row)
def append():
 with claude.open('a') as f:f.write('{"synthetic_uningested":true}\n')
compare('append-without-ingest',append)
compare('truncate-without-ingest',lambda:claude.write_text(''))
def replace():
 old=pi.stat();spare=pi.with_name('replacement.tmp');spare.write_bytes(pi.read_bytes());os.utime(spare,ns=(old.st_atime_ns,old.st_mtime_ns));assert spare.stat().st_ino!=old.st_ino;os.replace(spare,pi)
compare('same-size-mtime-new-inode',replace)
compare('delete-without-ingest',lambda:codex.unlink())
def detach():
 SRC.rename(BASE/'detached-sources')
 # Only this fresh synthetic product sidecar is removed, never read/copied.
 Path(str(DB)+'.key').unlink()
compare('detached-sources-missing-key',detach)
summary={'head':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),'binary_sha256':sha(BIN),'initial_tokens':15,'initial_loop_episode_count':1,'initial_loop_full_count':3,'analytics_sha256':db_before,'checks':checks,'limits':'synthetic current-entry gap reproduction, no source-stat baseline or candidate annotation implementation, no owner source/key access; source edits are fixture setup between reads, not attributed to Dendro'}
(BASE/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
print(json.dumps({'head':summary['head'],'binary_sha256':summary['binary_sha256'],'checks':[{'case':r['case'],'report_equal':r['report_exactly_equal'],'trace_equal':r['trace_exactly_equal'],'db_unchanged':r['analytics_bytes_unchanged'],'states':sorted(set(s['current_state'] for s in r['source_states']))} for r in checks]},indent=2))
