from pathlib import Path
import csv,json,hashlib,re,subprocess,shutil
out=Path('docs/audit/probes/native-foundation/results-2026-11-11-socket')
summary={}
for cpu in (2,4):
 d=out/f'final-{cpu}cpu';rows=list(csv.DictReader((d/'results.csv').open()))
 passed=[r for r in rows if r['acceptance']=='PASS'];failed=[r for r in rows if r['acceptance']=='FAIL'];build=[r['probe'] for r in rows if r['acceptance']=='BUILD_FAIL']
 assert len(passed)==68 and not failed,(cpu,failed)
 assert (d/'faults.log').read_bytes()==b''
 serial=(d/'serial.log').read_text()
 assert len(re.findall(r'\[NATIVE-C-INET\] before=0 after=0',serial))==68
 assert set(map(int,re.findall(r'\[NATIVE-C-REAP-END\] case=(\d+).*owned_frames=0 fds=0 stack=0',serial)))==set(range(68))
 for name in ('socket','inet-socket-contract','inet-socket-reap','epoll-lifecycle'):
  assert (d/(name+'.stdout.txt')).read_bytes()==b''
  assert (d/(name+'.stderr.txt')).read_bytes()==b''
 summary[str(cpu)]={'ram_mib':2048,'pass':len(passed),'guest_fail':len(failed),'historical_build_fail':build,'fault_bytes':0,'clean_reaps':68,'inet_reference_baselines_restored':68}
 fp=(out/f'native-std-fp-{cpu}cpu.log').read_text()
 assert '[NATIVE-STD] x87/XMM/MXCSR preemption isolation OK' in fp
 assert '[NATIVE-STD] native PDF parser and vector raster OK' in fp
 assert not re.search(r'KERNEL PANIC|\[EXCEPTION\]|\[FAULT\]',fp)
for name in ('socket','inet-socket-contract','inet-socket-reap','epoll-lifecycle'):
 assert (out/'final-2cpu'/(name+'.elf')).read_bytes()==(out/'final-4cpu'/(name+'.elf')).read_bytes()
summary['fp']={'2':'PASS','4':'PASS'}
for name in ('signal','pthread','poll','host','kernel','overlay','foundation'):
 assert (out/(name+'-driver.log')).read_text().rstrip().endswith('exit=0'),name
summary['regressions']='signal/pthread/poll ABI, host119, kernel, overlay, FP2/4 PASS'
summary['workspace']='FAIL: pre-existing kernel-target std/serde_core errors'
probes=json.loads((out/'historical-sha256.json').read_text())
for name,digest in probes.items():
 assert hashlib.sha256(Path(name).read_bytes()).hexdigest()==digest
 assert Path(name).read_bytes()==subprocess.check_output(['git','show','HEAD:'+name])
(out/'acceptance-summary.json').write_text(json.dumps(summary,indent=2)+'\n')
print(json.dumps(summary,indent=2))
