import collections,csv,hashlib,json,re,subprocess
from pathlib import Path
OUT=Path(__file__).resolve().parent
F=OUT/'final'
summary={'matrix':json.loads((F/'matrix-ledger.json').read_text()),'suites':{},'network_runs':{},'focused':{},'preservation':{}}
for cpu in [2,4]:
 rows=list(csv.DictReader((F/f'suite-{cpu}/results.csv').open()))
 summary['suites'][str(cpu)]={'total':len(rows),'acceptance_counts':dict(collections.Counter(r['acceptance'] for r in rows)),'nonpass':[r for r in rows if r['acceptance']!='PASS']}
for p in sorted(F.glob('*/*summary.json')):
 if p.name not in ['pressure-summary.json','poll-summary.json','external-summary.json']: continue
 s=json.loads(p.read_text());serial=p.parent/'guest/serial.log'
 if not serial.exists(): serial=p.parent/'serial.log'
 text=serial.read_text(errors='replace') if serial.exists() else ''
 summary['network_runs'][p.parent.name]={k:v for k,v in s.items() if k in ['result','checks','cpus','memory_mib','guest_pass_line','host_peer_exit','host_peer_pass_line','host_peer_ready','guest_stdout']}
 summary['network_runs'][p.parent.name]['support_exit']=re.findall(r'\[NATIVE-C-SERVICE-EXIT\] code=(-?\d+)',text)
 summary['network_runs'][p.parent.name]['resource_records']=[line for line in text.splitlines() if '[NATIVE-C-RESULT]' in line or '[NATIVE-C-INET]' in line or '[NATIVE-C-SUPPORT-REAP]' in line]
for name in ['lt-2-final','lt-4-final','revalidation-2','revalidation-4','inet-lt-2','inet-lt-4']:
 r=json.loads((F/name/'results.json').read_text()); t=(F/name/'serial.log').read_text(errors='replace')
 summary['focused'][name]={'results':r,'resources':[line for line in t.splitlines() if '[NATIVE-C-RESULT]' in line or '[NATIVE-C-INET]' in line]}
def digest(p):return hashlib.sha256(p.read_bytes()).hexdigest()
for label,manifest in [('incoming','incoming-preservation-hashes.json'),('source','final/source-hashes.json')]:
 d=json.loads((OUT/manifest).read_text()); summary['preservation'][label+'_mismatches']=[name for name,h in d.items() if not Path(name).is_file() or digest(Path(name))!=h]
summary['preservation']['head']=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip()
summary['preservation']['incoming_lock_diff_unchanged']=subprocess.check_output(['git','diff','--','tools/native-c/Cargo.lock'])==(OUT/'incoming-lock.diff').read_bytes()
archives=[Path('target/native-c/sysroot/lib/libsunlight_native_c.a'),Path('target/native-c/x86_64-sunlightos/release/libsunlight_native_c.a'),F/'libsunlight_native_c.a']
summary['archives']={str(p):digest(p) for p in archives}
summary['stable_archive_across_jobs']=all(r['archive_before']==r['archive_after']=='f1fe24486aa0152688408c16cb3ddddb793ef1024d582111f92cd33da4787441' for r in summary['matrix'] if r['name']!='build-archive')
(F/'acceptance-summary.json').write_text(json.dumps(summary,indent=2)+'\n')
(F/'final-status.txt').write_text(subprocess.check_output(['git','status','--short'],text=True))
(F/'final-diff-stat.txt').write_text(subprocess.check_output(['git','diff','--stat'],text=True))
print(json.dumps({k:summary[k] for k in ['suites','preservation','archives','stable_archive_across_jobs']},indent=2))
