from pathlib import Path
import importlib.util,json,os,shutil,datetime,gzip,re
R=Path('/home/ehsantor/Projects/sunlightos-kernel');O=R/'docs/audit/sunfs/sfs-5b4'
spec=importlib.util.spec_from_file_location('v',R/'tools/sunfs5b4_validate.py');v=importlib.util.module_from_spec(spec);spec.loader.exec_module(v)
old=O/'raw/regression-20261115T083526.956696Z';raw=O/'raw'/('regression-'+datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ'));raw.mkdir()
before=v.sources();assert before==json.loads((old/'source-before.json').read_text()),'Debug source changed; do not reuse'
v.put(raw/'source-before.json',before)
os.sched_setaffinity(0,{min(os.sched_getaffinity(0))})
env=dict(os.environ,CARGO_BUILD_JOBS='1',RUST_TEST_THREADS='1',CARGO_NET_OFFLINE='true')
v.put(raw/'runner.json',dict(helper_sha256=v.sha(Path(__file__)),build_cpu_affinity=list(os.sched_getaffinity(0)),environment={k:env[k] for k in ['CARGO_BUILD_JOBS','RUST_TEST_THREADS','CARGO_NET_OFFLINE']},execution='sequential commands; reuse explicitly identified earlier debug run on identical source'))
shutil.copytree(old/'debug',raw/'debug');shutil.copyfile(old/'hosted-debug.log',raw/'hosted-debug.log')
debug=json.loads((old/'commands.jsonl').read_text().splitlines()[0]);assert debug['returncode']==0
(raw/'commands.jsonl').write_text(json.dumps(debug)+'\n')
v.put(raw/'debug-reuse.json',dict(original=str(old.relative_to(O)),log_sha256=v.sha(old/'hosted-debug.log'),source_before_sha256=v.sha(old/'source-before.json'),source_matches_current=True,qualification='Earlier debug command executed in this milestone, not re-executed after cargo clean. Original full sweep interrupted; current complete source inventory matches its starting inventory.'))
results=[debug]
try:
 for sub,var in [('sfs1','SUNFS_EVIDENCE_DIR'),('sfs2','SUNFS_SFS2_EVIDENCE_DIR'),('sfs3','SUNFS_SFS3_EVIDENCE_DIR'),('sfs4','SUNFS_SFS4_EVIDENCE_DIR')]:
  d=raw/'release'/sub;d.mkdir(parents=True);env[var]=str(d)
 argv=['cargo','test','-p','sunlight-sunfs','-p','sunlight-sunfs-lab','-p','sunlight-sunfs-check','--offline','--locked','--target','x86_64-unknown-linux-gnu','--release']
 row=v.command(argv,raw,'hosted-release',env=env);results.append(row)
 if row['returncode']!=0:raise RuntimeError(row)
 for p in (raw/'release').rglob('*'):
  if p.is_file() and p.suffix in ('.img','.trace','.ownership'):
   p.with_suffix(p.suffix+'.gz').write_bytes(gzip.compress(p.read_bytes(),mtime=0));p.unlink()
 for name,argv in [
 ('storage',['cargo','test','-p','sunlight-block','-p','sunlight-fat','-p','sunlight-fs','-p','sunlight-virtio','--lib','--offline','--locked','--target','x86_64-unknown-linux-gnu']),
 ('no-std',['cargo','check','-p','sunlight-sunfs','-p','sunlight-block','-p','sunlight-virtio','--offline','--locked','--target','x86_64-unknown-none']),
 ('kernel-lifecycle',['cargo','test','-p','sunlight-kernel','--lib','--offline','--locked','--target','x86_64-unknown-linux-gnu']),
 ('kernel-default-build',['cargo','build','-p','sunlight-kernel','--offline','--locked']),
 ('format',['rustfmt','--edition','2021','--check','sunlight-virtio/src/blk.rs','kernel/src/sfs_reset.rs','kernel/src/sfs5b4_test.rs','kernel/build.rs']),
 ('clippy',['cargo','clippy','-p','sunlight-virtio','-p','sunlight-block','--lib','--offline','--locked','--target','x86_64-unknown-none']),
 ('whitespace',['git','diff','--check']),
 ('diagnostic',['cargo','build','-p','sunlight-kernel','--offline','--locked','--features','sfs5b4-test'])]:
  row=v.command(argv,raw,name,env=env)
  if len(results)<6:results.append(row)
  if row['returncode']!=0:raise RuntimeError(row)
finally:
 v.put(raw/'hosted-result.json',results)
 after=v.sources();v.put(raw/'source-after.json',after);changed=[n for n in set(before)|set(after) if before.get(n)!=after.get(n)];v.put(raw/'input-integrity.json',dict(status='PASS' if not changed else 'FAIL',changed=changed))
 print(raw,flush=True)
