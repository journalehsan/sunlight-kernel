import subprocess,pathlib,json
out=pathlib.Path(__file__).resolve().parent/'public';out.mkdir(exist_ok=False)
ledger=[]
def run(name,args):
 cmd=['timeout','--kill-after=10','300',*map(str,args)]
 with (out/(name+'.log')).open('w') as f:r=subprocess.run(cmd,stdout=f,stderr=subprocess.STDOUT)
 ledger.append(dict(name=name,command=cmd,exit=r.returncode));(out/'commands.json').write_text(json.dumps(ledger,indent=2));return r.returncode
elf=out/'public.elf'
rc=run('link',['python3','tools/native-c/cc.py','-Wall','-Wextra','-Werror','tools/native-c/tests/unix-replacement-public.c','-Wl,-Map='+str(out/'public.map'),'-o',elf])
if not rc:
 for cpu in [2,4]:run('public-'+str(cpu),['python3','tools/native-c/guest.py',elf,'--cpus',str(cpu),'--timeout','70','--expect-exit','0','--out',out/('public-'+str(cpu))])
