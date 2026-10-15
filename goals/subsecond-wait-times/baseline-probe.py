"""Analyst-only synthetic public-journey probe; no owner sources or keys."""
from pathlib import Path
import hashlib,json,sqlite3,subprocess
BASE=Path(__file__).resolve().parent
SRC=BASE/'baseline-probe'/'source'/'openrig.sqlite'
DB=BASE/'baseline-probe'/'analytics'/'dendro.duckdb'
BIN=Path('target/debug/dendro').resolve()
def digest(p):
    h=hashlib.sha256()
    with p.open('rb') as f:
        for chunk in iter(lambda:f.read(1024*1024),b''): h.update(chunk)
    return h.hexdigest()
assert not SRC.exists() and not DB.exists(), 'Preserve probe; do not overwrite an existing run'
SRC.parent.mkdir(parents=True)
DB.parent.mkdir(parents=True)
c=sqlite3.connect(SRC)
c.executescript('''CREATE TABLE rigs(id TEXT,name TEXT);
CREATE TABLE events(seq INTEGER,type TEXT,payload TEXT,created_at TEXT);
CREATE TABLE queue_transitions(transition_id INTEGER,qitem_id TEXT,ts TEXT,state TEXT,actor_session TEXT,closure_reason TEXT,closure_target TEXT,transition_note TEXT);
CREATE TABLE queue_items(qitem_id TEXT,destination_session TEXT,ts_updated TEXT,chain_of_record TEXT,handed_off_from TEXT);
''')
c.execute('INSERT INTO rigs VALUES (?,?)',('probe-rig-id','probe-rig'))
events=[]
def event(kind,payload,at):
    events.append((len(events)+1,kind,json.dumps(payload),at))
cases=[('reported','00.000000','24.734000'),('odd','00.000000','00.049000'),('odd','00.000000','00.051000'),('odd','00.000000','00.154000'),('even','00.000000','00.049000'),('even','00.000000','00.149000'),('zero','00.100000','00.100000'),('micro','00.000000','00.000001'),('nano','00.000000000','00.000000999'),('missing','00.000000',None),('invalid','00.000000','not-a-time'),('negative','01.000000','00.999999')]
for idx,(group,start,end) in enumerate(cases,1):
    a,b='sender-'+group,'receiver-'+group
    for seat in (a,b):
        event('agent.session_identity',{'sessionId':seat+'-session','runtime':'claude-code','sessionName':seat,'rigId':'probe-rig-id'},'2026-10-14 00:00:00')
    q='item-'+str(idx)
    at='2026-10-14T00:00:'+start+'Z'
    c.execute('INSERT INTO queue_items VALUES (?,?,?,?,?)',(q,b,at,'[]',None))
    event('queue.created',{'qitemId':q,'sourceSession':a,'destinationSession':b},'2026-10-14 00:00:00')
    c.execute('INSERT INTO queue_transitions VALUES (?,?,?,?,?,?,?,?)',(idx*10,q,at,'done',a,'handed_off_to',b,None))
    if end is not None:
        claim=('2026-10-14T00:00:'+end+'Z') if end!='not-a-time' else end
        c.execute('INSERT INTO queue_transitions VALUES (?,?,?,?,?,?,?,?)',(idx*10+1,q,claim,'in-progress',b,None,b,None))
c.executemany('INSERT INTO events VALUES (?,?,?,?)',events)
c.commit();c.close()
before=digest(SRC)
def run(args,name):
    r=subprocess.run([str(BIN),*args],text=True,capture_output=True)
    (BASE/'baseline-probe'/f'{name}.stdout').write_text(r.stdout)
    (BASE/'baseline-probe'/f'{name}.stderr').write_text(r.stderr)
    assert r.returncode==0,(name,r.returncode,r.stderr)
    return json.loads(r.stdout)
run(['run','--db',str(DB),'--root','openrig='+str(SRC)],'ingest')
filters=['--db',str(DB),'--rig','probe-rig','--from','2026-10-14T00:00:00Z','--to','2026-10-14T00:01:00Z']
report=run(['report','handoffs',*filters],'report')
trace=run(['trace','--metric-id','medians',*filters],'trace')
assert report['payload']['pair_medians']==trace['value']==trace['inputs']['pair_medians']
summary={'source_sha256_before':before,'source_sha256_after':digest(SRC),'binary_sha256':digest(BIN),'head':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),'rows':report['payload']['pair_medians'],'trace_rows':trace['inputs']['rows'],'parity':True}
assert before==summary['source_sha256_after']
(BASE/'baseline-probe'/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
print(json.dumps({'head':summary['head'],'rows':summary['rows'],'parity':True,'source_unchanged':True},indent=2))
