from pathlib import Path
import csv,hashlib,json,subprocess
root=Path.cwd();out=Path(__file__).resolve().parent
expected_history=b'exception OK\nTLS OK\nstatic initialization OK\natexit OK\n'
expected_lifecycle=b'global-init\nworker:TLS\nTLS-done\nmain:global:TLS\nTLS-done\natexit\nglobal-done\n'
summary={}
for cpus in (2,4):
 base=out/f'accepted-{cpus}cpu';rows=list(csv.DictReader((base/'results.csv').open()))
 built=[r for r in rows if r['build']=='PASS']
 assert len(built)==87 and all(r['acceptance']=='PASS' for r in built)
 original=next(r for r in rows if r['probe']=='cpp-runtime')
 assert original['acceptance']=='LINK_FAIL' and original['guest_run']=='NOT_RUN'
 assert (base/'cpp-runtime-native.stdout.txt').read_bytes()==expected_history
 assert (base/'cpp-runtime-native.stderr.txt').read_bytes()==b''
 assert not (base/'faults.log').read_bytes()
 for mode in ('uncaught','destructor','worker','storage'):
  row=next(r for r in rows if r['probe']=='cxx-runtime-'+mode)
  assert row['exit']=='134' and (base/('cxx-runtime-'+mode+'.stdout.txt')).read_bytes()==b'armed\n'
  assert not (base/('cxx-runtime-'+mode+'.stderr.txt')).read_bytes()
 life=out/f'lifecycle-{cpus}cpu';lrows=list(csv.DictReader((life/'results.csv').open()))
 assert len(lrows)==1 and lrows[0]['acceptance']=='PASS' and lrows[0]['exit']=='0'
 assert (life/'elf.stdout.txt').read_bytes()==expected_lifecycle
 assert not (life/'elf.stderr.txt').read_bytes() and not (life/'faults.log').read_bytes()
 summary[str(cpus)]={'ram_mib':2048,'built_native_cases':87,'native_pass':87,'configured_historical':'PASS','original_command':'LINK_FAIL; runtime NOT_RUN','lifecycle':'PASS exact output and zero live wrapped allocations','expected_fatal_cases':4,'faults':'none'}
probe_paths=[p for p in subprocess.check_output(['git','ls-files','docs/audit/probes'],text=True).splitlines() if p.endswith(('.c','.cpp','.py'))]
for f in probe_paths:
 assert Path(f).read_bytes()==subprocess.check_output(['git','show','HEAD:'+f]),f
summary['historical_integrity']={'files':len(probe_paths),'result':'PASS'}
(out/'acceptance-summary.json').write_text(json.dumps(summary,indent=2)+'\n')
print(json.dumps(summary,indent=2))
