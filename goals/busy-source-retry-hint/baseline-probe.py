"""Synthetic analyst diagnostic probe, never an owner source or rate benchmark."""
from pathlib import Path
import hashlib,json,os,sqlite3,subprocess,threading,time
BASE=Path(__file__).resolve().parent/'baseline-probe'
SRC=BASE/'source'/'openrig.sqlite'
OUT=BASE/'existing'/'dendro.duckdb'
BIN=Path('target/debug/dendro').resolve()
assert not BASE.exists(), 'Preserve original evidence: use a fresh probe destination'
SRC.parent.mkdir(parents=True)
OUT.parent.mkdir(parents=True)
c=sqlite3.connect(SRC)
c.executescript('CREATE TABLE events(seq INTEGER,type TEXT,payload TEXT,created_at TEXT); CREATE TABLE padding(data BLOB);')
c.execute('INSERT INTO events VALUES (?,?,?,?)',(1,'seat.activity_changed',json.dumps({'sessionName':'synthetic-seat','state':'running','at':'2026-10-14T00:00:00Z','unexpected':'SYNTHETIC_30_CONTENT_CANARY sk-SYNTHETIC30FAKEONLY'}),'2026-10-14 00:00:00'))
for _ in range(64): c.execute('INSERT INTO padding VALUES (zeroblob(?))',(256*1024,))
c.commit();c.close()
def digest(path):
 if not path.exists(): return None
 h=hashlib.sha256()
 with path.open('rb') as f:
  for part in iter(lambda:f.read(1024*1024),b''): h.update(part)
 return h.hexdigest()
def cli(name,db=OUT,root=SRC):
 r=subprocess.run([str(BIN),'run','--db',str(db),'--root','openrig='+str(root)],capture_output=True,text=True,timeout=90)
 (BASE/f'{name}.stdout').write_text(r.stdout)
 (BASE/f'{name}.stderr').write_text(r.stderr)
 return {'name':name,'exit':r.returncode,'stdout':r.stdout,'stderr':r.stderr}
source_before=digest(SRC)
rows=[cli('stable-before')]
assert rows[-1]['exit']==0,rows[-1]
analytics_before=digest(OUT)
# Only this synthetic producer changes source mtime; file bytes remain static.
stop=threading.Event(); count=[0]
def producer():
 while not stop.is_set():
  now=time.time_ns()
  os.utime(SRC,ns=(now,now)); count[0]+=1
  stop.wait(0.0001)
t=threading.Thread(target=producer)
t.start()
try:
 rows.append(cli('busy-existing'))
 rows.append(cli('busy-fresh',BASE/'fresh'/'dendro.duckdb'))
finally:
 stop.set(); t.join()
analytics_after=digest(OUT)
stages=list(BASE.glob('**/.dendro-stage-*'))
rows.append(cli('stable-after'))
corrupt=BASE/'corrupt'/'openrig.sqlite'; corrupt.parent.mkdir()
corrupt.write_bytes(b'SYNTHETIC_30_CORRUPT_DATABASE_CONTENT')
corrupt_before=digest(corrupt)
rows.append(cli('corrupt-control',BASE/'corrupt-output'/'dendro.duckdb',corrupt))
summary={'head':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),'binary_sha256':digest(BIN),'source_sha256_before':source_before,'source_sha256_after':digest(SRC),'source_bytes_unchanged':source_before==digest(SRC),'producer':'synthetic mtime-only changes; no byte edits, no live source, not a commit-rate or performance experiment','producer_changes':count[0],'analytics_sha256_before_refusal':analytics_before,'analytics_sha256_after_refusal':analytics_after,'analytics_unchanged_by_refusal':analytics_before==analytics_after,'fresh_database_exists_after_refusal':(BASE/'fresh'/'dendro.duckdb').exists(),'private_stages_after_refusal':[str(p) for p in stages],'corrupt_source_unchanged':corrupt_before==digest(corrupt),'rows':rows}
(BASE/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
print(json.dumps(summary,indent=2))
