"""Analyst investigation only. Synthetic roots, payload-free existing observer."""
from pathlib import Path
import hashlib,importlib.util,json,os,sqlite3,subprocess,sys
sys.dont_write_bytecode=True
REPO=Path.cwd().resolve(); BASE=Path(__file__).resolve().parent/'baseline-probe'
BIN=REPO/'target/debug/dendro'; STRACE=REPO/'.agent-runs/tools/strace/bin/strace'
OBSERVER=REPO/'.agent-runs/issues/46/oracles/v1/tools/observe.py'
assert not BASE.exists(),'Preserve original probe; use a fresh destination'
BASE.mkdir()
def digest(p):
 h=hashlib.sha256()
 with p.open('rb') as f:
  for block in iter(lambda:f.read(1048576),b''):h.update(block)
 return h.hexdigest()
provenance=json.loads((REPO/'.agent-runs/tools/strace/provenance.json').read_text())
for item in provenance['files']:assert digest(REPO/item['path'])==item['sha256'],item['path']
controls=subprocess.run([sys.executable,str(OBSERVER.with_name('parser-selftest.py'))],capture_output=True,text=True,env={**os.environ,'PYTHONDONTWRITEBYTECODE':'1'})
(BASE/'observer-controls.stdout').write_text(controls.stdout);(BASE/'observer-controls.stderr').write_text(controls.stderr)
assert controls.returncode==0,controls.stderr
spec=importlib.util.spec_from_file_location('qa46_observe',OBSERVER); obs=importlib.util.module_from_spec(spec);spec.loader.exec_module(obs)
RAW='read,write,pread64,pwrite64,readv,writev,preadv,pwritev,preadv2,pwritev2'
FILTER='%file,%memory,%process,eventfd,eventfd2,epoll_create,epoll_create1,socket,socketpair,pipe,pipe2,accept,accept4,close,dup,dup2,dup3,fcntl,flock,ftruncate,fallocate,copy_file_range,sendfile,splice,fsync,fdatasync,'+RAW
summaries=[]
for case,suffix in [('lock-main',''),('lock-wal','-wal'),('lock-shm','-shm'),('safe-distinct',None)]:
 d=BASE/case; fixture=d/'fixture'; srcdir=fixture/'source'; out=fixture/'output'
 srcdir.mkdir(parents=True);out.mkdir();source=srcdir/'openrig.sqlite';dest=out/'analytics.duckdb';lock=out/'.dendro-output.lock'
 c=sqlite3.connect(source)
 c.execute('CREATE TABLE events(seq INTEGER,type TEXT,payload TEXT,created_at TEXT)')
 c.execute('INSERT INTO events VALUES(?,?,?,?)',(1,'seat.activity_changed',json.dumps({'sessionName':'probe-seat','state':'running','at':'2026-10-15T00:00:00Z','unexpected':'SYNTHETIC_47_CONTENT_CANARY sk-SYNTHETIC47FAKEONLY'}),'2026-10-15 00:00:00'))
 c.commit();c.execute('PRAGMA journal_mode=WAL');c.execute('PRAGMA wal_autocheckpoint=0');c.execute('PRAGMA wal_checkpoint(TRUNCATE)')
 c.execute('INSERT INTO events VALUES(?,?,?,?)',(2,'seat.activity_changed',json.dumps({'sessionName':'probe-seat','state':'idle','at':'2026-10-15T00:00:01Z'}),'2026-10-15 00:00:01'));c.commit()
 protected=[source,Path(str(source)+'-wal'),Path(str(source)+'-shm')]
 if suffix is not None:os.link(Path(str(source)+suffix),lock)
 def state():
  rows={}
  for p in sorted(fixture.rglob('*')):
   st=p.lstat();row={'dev':st.st_dev,'ino':st.st_ino,'nlink':st.st_nlink,'mode':st.st_mode,'size':st.st_size,'mtime_ns':st.st_mtime_ns}
   if p.is_file() and not p.name.endswith('.key'):row['sha256']=digest(p)
   if p.name.endswith('.key'):row['content']='excluded; never read'
   rows[str(p.relative_to(fixture))]=row
  return rows
 before=state(); identities={str(fixture/k):[v['dev'],v['ino']] for k,v in before.items()}
 argv=[str(BIN),'run','--db',str(dest),'--root','openrig='+str(source)]
 trace=d/'trace.syscalls'; actual=[str(STRACE),'-f','-yy','-s','4096','-o',str(trace),'-e','trace='+FILTER,'-e','raw='+RAW,*argv]
 r=subprocess.run(actual,cwd=fixture,capture_output=True,text=True,timeout=90,close_fds=True)
 after=state()
 receipt={'cwd':str(fixture),'argv':argv,'observer_argv':actual,'protected_paths':[str(p) for p in protected],'protected_dirs':[],'identity_paths':identities,'protected_inodes':[identities[str(p)] for p in protected],'before':before,'after':after,'exit':r.returncode,'stdout':r.stdout,'stderr':r.stderr,'producer':'synthetic SQLite connection quiescent throughout before/CLI/after; close/checkpoint only after manifests; child close_fds=True, source producer descriptors not inherited'}
 result=obs.analyze(receipt,trace.read_text().splitlines())
 (d/'receipt.json').write_text(json.dumps(receipt,indent=2)+'\n');(d/'operations.json').write_text(json.dumps(result,indent=2)+'\n')
 (d/'stdout').write_text(r.stdout);(d/'stderr').write_text(r.stderr)
 source_keys=[str(p.relative_to(fixture)) for p in protected]
 unchanged=all(before[k]==after[k] for k in source_keys)
 writable=[x for x in result['violations'] if x['kind']=='write-capable-open-attempt']
 mutators=[x for x in result['violations'] if x['kind'] in ['source-fd-mutator-attempt','source-path-mutator-attempt','shared-writable-mapping-attempt','shared-writable-mapping-upgrade-attempt']]
 summaries.append({'case':case,'exit':r.returncode,'stderr':r.stderr,'primary_distinct':not dest.exists() or all((dest.stat().st_dev,dest.stat().st_ino)!=(p.stat().st_dev,p.stat().st_ino) for p in protected),'lock_identity':identities.get(str(lock)),'target_identity':identities.get(str(source)+suffix) if suffix is not None else None,'source_state_unchanged':unchanged,'tree_unchanged':before==after,'write_capable_source_opens':writable,'observed_source_mutators':mutators,'observer_unknown':result['unknown'],'normal_descendant_exit_coverage':result['normal_exit_observed'],'canaries_absent_from_cli':all(v not in r.stdout+r.stderr for v in ['SYNTHETIC_47_CONTENT_CANARY','sk-SYNTHETIC47FAKEONLY']),'primary_db_exists':dest.exists()})
 c.close()
summary={'head':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),'binary_sha256':digest(BIN),'observer_sha256':digest(OBSERVER),'observer_provenance':provenance,'parser_controls':controls.stdout,'raw_buffer_capture':False,'cases':summaries,'limits':'analyst entry-behavior investigation, not candidate QA/review; no real owner source/key or runtime frequency claim; counts apply only to recorded trace scope'}
(BASE/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
print(json.dumps({k:v for k,v in summary.items() if k not in ['observer_provenance']},indent=2))
