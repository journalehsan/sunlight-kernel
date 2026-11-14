from pathlib import Path
import csv,json,re,hashlib
E=Path(__file__).resolve().parent; checks=[]
def read(p): return json.loads((E/p).read_text())
def check(name,value):
 checks.append({'check':name,'pass':bool(value)})
def out(name):return (E/name/'elf.stdout.txt').read_text()
summary=read('validation-summary.json'); expected_negative={'exit-api-negative','exit-link-negative'}
check('every execution/build command has expected status',all(r['exit']==(1 if r['name'] in expected_negative else 0) for r in summary['runs']))
for cpu in [1,2,4]:
 check(f'cpu{cpu} 256 stale public identities rejected','HANDLE 256 reuse cycles' in out(f'handle-reuse-cpu-{cpu}'))
 check(f'cpu{cpu} admission64/exhaustion/recovery','65th create=EAGAIN' in out(f'handle-admission-cpu-{cpu}') and '64 stale identities rejected' in out(f'handle-admission-cpu-{cpu}'))
 check(f'cpu{cpu} actual reserved-join interval and registry empty','CLAIM 32 native reserved-join overlaps' in out(f'handle-claim-cpu-{cpu}') and 'joining=1 target=live detach=EINVAL second_join=EINVAL' in out(f'handle-claim-cpu-{cpu}') and 'registry controls=0 after each round' in out(f'handle-claim-cpu-{cpu}'))
 initial=out(f'initial-exit-cpu-{cpu}');serial=(E/f'initial-exit-cpu-{cpu}/serial.log').read_text()
 check(f'cpu{cpu} E1 negative contract reproduced',all(s in initial for s in ['I1 worker independently alive','I2 initial TLS cleanup entered','I3 initial calling pthread_exit']) and 'I4 worker survival' not in initial and 'I5 worker TLS' not in initial and 'reason=address-space-owner-exit' in serial)
 for mode,code in [(0,17),(1,23),(2,29),(3,137)]:
  name=f'exit-mode-{mode}-cpu-{cpu}';result=read(name+'/results.json')['elf'];stdout=result['stdout'];trace=(E/name/'serial.log').read_text()
  check(f'cpu{cpu} exit mode{mode} exact callback/exit/resource contract',result['pass_'] and result['exit']==code and stdout.count('EXIT live worker established')==1 and stdout.count('EXIT atexit callback')==(1 if mode in [0,1] else 0) and re.search(r'\[NATIVE-C-REAP-END\].*owned_frames=0 fds=0 stack=0',trace))
  if mode==3:
   check(f'cpu{cpu} force transition and exactly one owner/borrower reap','reason=signal(SIGKILL)' in trace and 'reason=address-space-owner-exit' in trace and len(re.findall(r'\[SCHED\] process_reaped pid=100 ',trace))==1 and len(re.findall(r'\[SCHED\] process_reaped pid=101 ',trace))==1)
 wave=read('detached-reclamation.json')['waves'][str(cpu)];snaps=wave['snapshots']
 for phase in ['all-kernel-reaped','bounded-quiescence']:
  selected=[s for s in snaps if s['phase']==phase]
  check(f'cpu{cpu} four detached waves {phase}',len(selected)==4 and all((s['queued'],s['control_bytes'],s['stack_bytes'],s['tls_bytes'])==(12,1152,25165824,98304) for s in selected))
 check(f'cpu{cpu} explicit wave sweep reaches zero',snaps[-1]['queued']==0)
 cap=out(f'detached-capacity-cpu-{cpu}')
 check(f'cpu{cpu} actual64 detached queue idle retention and complete sweep','phase=after-64-detaches queued=64 control_bytes=6144 stack_bytes=134217728 tls_bytes=524288' in cap and 'phase=quiescent-without-create queued=64' in cap and 'phase=explicit-create-sweep queued=0 control_bytes=0 stack_bytes=0 tls_bytes=0' in cap)
 check(f'cpu{cpu} callback/delete/reuse interval completes',all(marker in out(f'tls-controlled-cpu-{cpu}') for marker in ['T1 holder=','T2 deleting=','T3 old_get=NULL old_set=EINVAL reused_get=NULL new_set=0','T4 callback=','T5 delete/reuse while callback held','T6 selected callback completed after delete/reuse','T7 joins=2 callbacks=1 outstanding_joinable=0 keys_live=0']))
 suite=read('regression-results.json')['suites'][str(cpu)]
 check(f'cpu{cpu} every73C/21C++ guest cell passes',suite['status']=='PASS' and suite['counts']=={'C':{'total':73,'passed':73},'C++':{'total':21,'passed':21}} and all(r['acceptance']=='PASS' for r in suite['individual_cases']))
for prefix,folder in [('accounting','detached-accounting'),('capacity','detached-capacity'),('claim','handle-claim')]:
 metadata=read(prefix+'-build.json');check(prefix+' alias binding verified',metadata['binding_verified'])
 for cpu in [1,2,4]:
  rows=list(csv.DictReader((E/f'{folder}-cpu-{cpu}/results.csv').open()))
  check(f'{prefix} cpu{cpu} ELF matches audited binding',len(rows)==1 and rows[0]['sha256']==metadata['elf_sha256'])
check('missing _Exit declaration diagnostic preserved',"undeclared function '_Exit'" in (E/'exit-api-negative.log').read_text())
check('missing _Exit export diagnostic preserved','undefined symbol: _Exit' in (E/'exit-link-negative.log').read_text())
for cpu,folder in [(2,'tcp-loopback'),(4,'tcp-loopback-cpu-4')]:
 p=read(folder+'/poll-summary.json');check(f'TCP cpu{cpu} live resource counters zero',p['result']=='PASS' and 'socket_live=0 listener_live=0 stream_live=0' in p['guest_stdout'])
check('only authorized two baseline source files changed',read('source-integrity.json')['status']=='PASS' and set(read('source-integrity.json')['changed_baseline_files'])=={'sunlight-libc/src/pthread.rs','sunlight-libc/include/pthread.h'})
check('raw vendor manifest mismatch distinguished from unchanged numerical files',read('vendor-integrity.json')['raw_R2A_manifest_comparison']=='FAIL' and read('vendor-integrity.json')['pinned_upstream_differences_excluding_manifest']==[] and read('vendor-integrity.json')['baseline_comparison_changed']==[])
check('strict race gates remain open',summary['gates']['G5']=='PARTIAL' and summary['gates']['G6']=='UNQUALIFIED' and summary['complete'] is False and summary['overall']=='PARTIAL')
for p in summary['required_artifacts']:check('required artifact '+p,(E/p).is_file())
result={'status':'PASS' if all(r['pass'] for r in checks) else 'FAIL','checks':checks,'scope':'Evidence consistency checks; do not replace native witnesses or qualify unobserved races'}
(E/'evidence-validation.json').write_text(json.dumps(result,indent=2)+'\n')
print(result['status'],len(checks),'checks');print([r for r in checks if not r['pass']]);raise SystemExit(result['status']!='PASS')
