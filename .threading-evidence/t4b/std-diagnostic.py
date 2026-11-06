import sys,json
from pathlib import Path
sys.path.insert(0,str(Path.cwd()/'tools'))
from threading_t4b_validate import ROOT,BASE,run
from datetime import datetime,timezone
folder=BASE/(datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')+'-std-diagnostic');folder.mkdir();result={'pass':False,'attempts':[]}
def command(name,args):
 ok,path=run('std-diagnostic-'+name,args);result['attempts'].append(str(path.relative_to(ROOT)));assert ok,path
try:
 command('supervisor',['env','RUSTFLAGS=-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone','cargo','build','-p','sunlightd','--release','--features','document-service-gate','--offline'])
 command('build',['env','SUNLIGHT_INJECT_PHASE=document-service','cargo','build','-p','sunlight-kernel','--bin','sunlight-kernel','--offline','--features','key_inject,document_service,native_document_service,futex_lifecycle_diag'])
 command('freeze',['objcopy','--strip-debug','target/x86_64-unknown-none/debug/sunlight-kernel',str(folder/'kernel.elf')])
 command('iso',['bash','tools/make_hybrid_iso.sh',str(folder/'kernel.elf'),str(folder/'boot.iso'),'target/limine',str(ROOT)])
 markers=['[NATIVE-STD] T4A mutex/condvar handoffs=1536 independent=4 progress/TLS/resources OK','[NATIVE-STD] collections/clocks/locks/TLS/join OK','[NATIVE-STD] T2 Hayro workers=4 concurrent render/join OK','[NATIVE-STD] shared descriptors/OOM/group exit OK','[DOCSERV] native Hayro IPC open/render/close OK']
 ok,path=run('std-diagnostic-12cpu',[],180,folder/'boot.iso',12,markers);result['attempt']=str(path.relative_to(ROOT));result['pass']=ok
finally:(folder/'result.json').write_text(json.dumps(result,indent=2)+'\n');print(folder)
