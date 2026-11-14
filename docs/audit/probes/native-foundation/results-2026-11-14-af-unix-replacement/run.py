import pathlib,subprocess,json,hashlib
root=pathlib.Path.cwd(); out=pathlib.Path(__file__).resolve().parent
ledger=[]
def run(name,args,seconds=300):
 cmd=['timeout','--kill-after=10',str(seconds),*map(str,args)]
 with (out/(name+'.log')).open('w') as f:
  r=subprocess.run(cmd,stdout=f,stderr=subprocess.STDOUT)
 ledger.append(dict(name=name,command=cmd,exit=r.returncode));(out/'commands.json').write_text(json.dumps(ledger,indent=2));return r.returncode
for mode in ['linux','native']:
 elf=out/(mode+'.elf')
 rc=run(mode+'-link',['clang','--target=x86_64-unknown-none-elf','-mno-red-zone','-ffreestanding','-fno-stack-protector','-fno-pie','-nostdlib','-static','-fuse-ld=lld','-Wall','-Wextra','-Werror',*(['-DNATIVE_ACTOR'] if mode=='native' else []),'tools/native-c/tests/unix-replacement.c','-Wl,-T,services/user-space.ld','-Wl,-Map='+str(out/(mode+'.map')),'-o',elf])
 if rc: continue
 if mode=='linux':
  b=bytearray(elf.read_bytes());b[7]=3;elf.write_bytes(b)
 for cpu in [2,4]:
  run(mode+'-'+str(cpu),['python3','tools/native-c/guest.py',elf,'--cpus',str(cpu),'--timeout','70','--expect-exit','0','--out',out/(mode+'-'+str(cpu)),*(['--linux-actor'] if mode=='linux' else [])])
