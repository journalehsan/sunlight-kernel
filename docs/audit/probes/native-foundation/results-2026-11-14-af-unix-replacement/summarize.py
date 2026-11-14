"""Independently classify every retained guest assertion and suite row."""
from pathlib import Path
import csv, hashlib, json, re, subprocess
p=Path(__file__).resolve().parent
results={}
def evaluate(key,folder,prefix=None,expected=None,suite=False):
    rows=json.loads((folder/'results.json').read_text())
    stdout=(folder/'elf.stdout.txt').read_text() if (folder/'elf.stdout.txt').exists() else ''
    serial=(folder/'serial.log').read_text(errors='replace')
    passes=[x for x in stdout.splitlines() if ': PASS ' in x]
    summaries=[x for x in passes if x.endswith('PASS complete') or x.startswith('dup2-replacement: PASS local tcp replacement ')]
    failures=re.findall(r'^.*: FAIL .*$',stdout,re.M)
    clean=re.findall(r'\[NATIVE-C-RESULT\].*',serial)
    inet=re.findall(r'\[NATIVE-C-INET\] before=(\d+) after=(\d+)',serial)
    with (folder/'results.csv').open() as f: cells=list(csv.DictReader(f))
    ok=bool(rows) and all(x['pass_'] for x in rows.values()) and len(cells)==len(rows) and all(x['build']=='PASS' and x['guest_run']=='PASS' and x['acceptance']=='PASS' for x in cells) and len(clean)==len(rows) and all('clean=1 ' in x for x in clean) and len(inet)==len(rows) and all(a==b for a,b in inet)
    if not suite:ok=ok and not failures
    if prefix:ok=ok and prefix+': PASS complete' in stdout
    if expected is not None:ok=ok and len(passes)-len(summaries)==expected
    if suite:ok=ok and len(rows)==94
    commands=json.loads((folder.parent/'commands.json').read_text())
    command=next(x for x in commands if x['name']==folder.name)
    if command['exit']==124:classification='TIMEOUT'
    else:classification='PASS' if ok and command['exit']==0 else 'FAIL'
    results[key]=dict(build='PASS',link='PASS',runtime=classification,assertions=len(passes)-len(summaries),completions=len(summaries),failures=failures,rows=len(rows),passed_rows=sum(bool(x['pass_']) for x in rows.values()),individual_pass_records=passes,kernel_cleanup=clean,inet_refs=inet,csv=cells)
for cpu in [2,4]:
    for mode in ['linux','native']:
        evaluate(f'initial-{mode}-{cpu}',p/f'{mode}-{cpu}','unix',200)
        evaluate(f'expanded-{mode}-{cpu}',p/'expanded'/f'{mode}-{cpu}','unix')
        evaluate(f'validated-{mode}-{cpu}',p/'validated'/f'{mode}-{cpu}','unix',508)
    evaluate(f'public-{cpu}',p/'public'/f'public-{cpu}','unix-public',136)
    for name,count,prefix in [('wrapper',406,'dup-runtime'),('linux-handler',413,'dup-runtime'),('linux-entry',44,'linux-entry'),('dup2-replacement',50,None),('epoll',None,None),('epoll-lifecycle',None,None),('inet-socket',None,None),('tcp-epoll-lt',None,None),('tcp-epoll-revalidation',None,None),('suite',None,None)]:
        evaluate(f'{name}-{cpu}',p/'regression'/f'{name}-{cpu}',prefix,count,name=='suite')
checks=json.loads((p/'regression/summary.json').read_text())
for key in ['native-build','cxx-checker','poll-abi','kernel-check','libc-check','network-check','diff-check']:
    results[key]=checks[key]
(p/'acceptance-summary.json').write_text(json.dumps(results,indent=2))
expected_fails={'expanded-native-2','expanded-native-4'}
assert {k for k,v in results.items() if v.get('runtime') in ['FAIL','TIMEOUT']}==expected_fails
assert all(results[k]['failures'].count('unix: FAIL replacement-preserves-descriptor-flags')==8 and results[k]['failures'].count('unix: FAIL complete')==1 and len(results[k]['failures'])==9 for k in expected_fails)
assert all(all(v=='PASS' for v in results[k].values()) for k in ['native-build','cxx-checker','poll-abi','kernel-check','libc-check','network-check','diff-check'])
for f in [Path('target/native-c/sysroot/lib/libsunlight_native_c.a'),Path('target/native-c/x86_64-sunlightos/release/libsunlight_native_c.a')]:
    assert hashlib.sha256(f.read_bytes()).hexdigest()=='2fb382ef6c2b83b48e034645a078d5e36e40f036d98b00a00eace35d222696c7'
assert hashlib.sha256(Path('target/x86_64-unknown-none/debug/sunlight-kernel').read_bytes()).hexdigest()=='97d9bb2204549ec7548b14fc6d2c90af0d3986dfdd5292aa5eec8ba21e931594'
assert subprocess.check_output(['git','diff','--binary'])==(p/'incoming.txt').read_bytes()
print('PASS: all final assertions, suite CSV/JSON rows, cleanup records, production hashes and incoming tracked diffs verified. Two expected harness failures preserved.')
