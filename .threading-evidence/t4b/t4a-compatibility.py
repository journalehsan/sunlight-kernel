import sys,json,hashlib
from pathlib import Path
sys.path.insert(0,str(Path.cwd()/'tools'))
import threading_t4a_validate as old
from threading_t4b_validate import ROOT,BASE,identity,run
old.BASE=BASE
from datetime import datetime,timezone
folder=BASE/(datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')+'-t4a-compatibility')
folder.mkdir();source=identity();(folder/'source.json').write_text(json.dumps(source,indent=2)+'\n');result={'pass':False,'attempts':[]}
def command(name,args):
 ok,path=run('t4a-compat-'+name,args);result['attempts'].append(str(path.relative_to(ROOT)));assert ok,path
try:
 command('build',['cargo','build','-p','sunlight-kernel','--bin','sunlight-kernel','--offline','--features','atomic_futex_test'])
 command('freeze',['objcopy','--strip-debug','target/x86_64-unknown-none/debug/sunlight-kernel',str(folder/'kernel.elf')])
 command('iso',['bash','tools/make_hybrid_iso.sh',str(folder/'kernel.elf'),str(folder/'boot.iso'),'target/limine',str(ROOT)])
 ok,path=old.run('t4a-compat-2cpu',[],300,folder/'boot.iso',2,[],True)
 result['guest']=str(path.relative_to(ROOT));result['pass']=ok and identity()==source
 result['kernel_sha256']=hashlib.sha256((folder/'kernel.elf').read_bytes()).hexdigest();result['iso_sha256']=hashlib.sha256((folder/'boot.iso').read_bytes()).hexdigest()
finally:
 (folder/'result.json').write_text(json.dumps(result,indent=2)+'\n');print(folder)
assert result['pass'],result
