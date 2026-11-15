from pathlib import Path
import subprocess,os,json,time,hashlib,gzip,datetime
R=Path('/home/ehsantor/Projects/sunlightos-kernel');O=R/'docs/audit/sunfs/sfs-5b4/raw';env=dict(os.environ,CARGO_BUILD_JOBS='1',CARGO_NET_OFFLINE='true')
D=O/('guest-recovery-'+datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ'));D.mkdir()
(D/'runner.json').write_text(json.dumps({'helper_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),'scope':'Existing guest build prerequisites only; no production source modifications'},indent=2)+'\n')
# Restrict compiler/build prerequisites only. Native QEMU tests run separately.
os.sched_setaffinity(0,{min(os.sched_getaffinity(0))})
rows=[]
for name,argv in [('limine',['bash','tools/setup_limine.sh',str(R/'target/limine'),'v8.x']),('yazi',['bash','tools/build_yazi.sh']),('kernel',['cargo','build','-p','sunlight-kernel','--offline','--locked','--features','sfs5b4-test'])]:
 start=time.monotonic()
 with (D/f'{name}.log').open('x') as log:p=subprocess.run(argv,cwd=R,env=env,stdout=log,stderr=subprocess.STDOUT,timeout=3600)
 rows.append({'name':name,'argv':argv,'returncode':p.returncode,'seconds':time.monotonic()-start,'environment':{k:env[k] for k in ['CARGO_BUILD_JOBS','CARGO_NET_OFFLINE']},'build_cpu_affinity':list(os.sched_getaffinity(0))})
 (D/'commands.json').write_text(json.dumps(rows,indent=2)+'\n')
 print(name,p.returncode,flush=True)
 if p.returncode:raise SystemExit(p.returncode)
elf=R/'target/x86_64-unknown-none/debug/sunlight-kernel'
(D/'recovered-guest.elf.gz').write_bytes(gzip.compress(elf.read_bytes(),mtime=0))
(O/'recovered-guest-identity.json').write_text(json.dumps({'sha256':hashlib.sha256(elf.read_bytes()).hexdigest(),'compressed_copy':str((D/'recovered-guest.elf.gz').relative_to(O)),'build_load':'one compiler job on one host CPU; native runner not affinity restricted'},indent=2)+'\n')
