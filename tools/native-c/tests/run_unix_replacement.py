#!/usr/bin/env python3
"""Bounded freestanding AF_UNIX transition probe; preserves each output directory."""
import pathlib,subprocess,json,hashlib,re,shutil
root=pathlib.Path.cwd(); import argparse
p=argparse.ArgumentParser();p.add_argument("--out",type=pathlib.Path,required=True);a=p.parse_args()
out=a.out.resolve()
out.mkdir(exist_ok=False)
ledger=[]
results={}
source=root/"tools/native-c/tests/unix-replacement.c"
shutil.copyfile(source,out/"fixture.c")
def run(name,args,seconds=300):
 cmd=['timeout','--kill-after=10',str(seconds),*map(str,args)]
 with (out/(name+'.log')).open('w') as f:
  r=subprocess.run(cmd,stdout=f,stderr=subprocess.STDOUT)
 ledger.append(dict(name=name,command=cmd,exit=r.returncode));(out/'commands.json').write_text(json.dumps(ledger,indent=2));return r.returncode
for mode in ['linux','native']:
 elf=out/(mode+'.elf')
 rc=run(mode+'-link',['clang','--target=x86_64-unknown-none-elf','-mno-red-zone','-ffreestanding','-fno-stack-protector','-fno-pie','-nostdlib','-static','-fuse-ld=lld','-Wall','-Wextra','-Werror',*(['-DNATIVE_ACTOR'] if mode=='native' else []),'tools/native-c/tests/unix-replacement.c','-Wl,-T,services/user-space.ld','-Wl,-Map='+str(out/(mode+'.map')),'-o',elf])
 results[mode]={"build":"PASS" if rc==0 else "BUILD_FAIL", "link":"PASS" if rc==0 else "TIMEOUT" if rc==124 else "LINK_FAIL"}
 if rc: continue
 if mode=='linux':
  b=bytearray(elf.read_bytes());b[7]=3;elf.write_bytes(b)
 for cpu in [2,4]:
  key=mode+"-"+str(cpu)
  rc=run(mode+'-'+str(cpu),['python3','tools/native-c/guest.py',elf,'--cpus',str(cpu),'--timeout','70','--expect-exit','0','--out',out/(mode+'-'+str(cpu)),*(['--linux-actor'] if mode=='linux' else [])])

  folder=out/key
  rows=json.loads((folder/'results.json').read_text()) if (folder/'results.json').exists() else {}
  stdout=(folder/'elf.stdout.txt').read_text() if (folder/'elf.stdout.txt').exists() else ''
  serial=(folder/'serial.log').read_text(errors='replace') if (folder/'serial.log').exists() else ''
  passes=re.findall(r'^unix: PASS (.*)$',stdout,re.M)
  fails=re.findall(r'^unix: FAIL (.*)$',stdout,re.M)
  cleanup=re.findall(r'\[NATIVE-C-RESULT\].*',serial)
  ok=rc==0 and rows and all(x['pass_'] for x in rows.values()) and passes.count('complete')==1 and not fails and cleanup and all('clean=1 ' in x for x in cleanup)
  results[key]=dict(runtime='PASS' if ok else 'TIMEOUT' if rc==124 else 'FAIL',assertions=len(passes)-passes.count('complete'),failures=fails,individual=passes,rows=rows,cleanup=cleanup)
  (out/'summary.json').write_text(json.dumps(results,indent=2))
(out/'hashes.json').write_text(json.dumps({str(f):hashlib.sha256(f.read_bytes()).hexdigest() for f in [source,*out.glob('*.elf'),*out.glob('*.map'),*out.glob('*/kernel.elf'),*out.glob('*/guest.iso')]},indent=2))
raise SystemExit(int(any(v.get('runtime',v.get('link'))!='PASS' for v in results.values())))
