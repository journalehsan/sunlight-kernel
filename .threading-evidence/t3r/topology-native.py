import sys,json,subprocess,time,hashlib
from pathlib import Path
from datetime import datetime,timezone
sys.path.insert(0,'tools')
from threading_t3_validate import identity
from threading_t3r_receipt import assess_migration
root=Path.cwd();base=root/'.threading-evidence/t3r'
migration=sorted(p for p in base.glob('*-migration') if (p/'result.json').exists())[-1]
iso=migration/'boot.iso'
p=base/(datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')+'-nonzero-bsp-attempt');p.mkdir()
(p/'source.json').write_text(json.dumps(identity(),indent=2)+'\n')
argv=['qemu-system-x86_64','-accel','tcg,thread=multi','-cpu','max,apic-id=42','-smp','2','-m','2048','-cdrom',str(iso),'-serial','file:'+str(p/'serial.log'),'-display','none','-nic','none','-device','virtio-rng-pci,disable-modern=on','-device','isa-debug-exit,iobase=0xf4,iosize=0x04','-no-reboot','-no-shutdown']
start=time.monotonic();r={'argv':argv,'iso_sha256':hashlib.sha256(iso.read_bytes()).hexdigest(),'requested_bsp_apic':42,'classification':'BLOCKED / NOT_RUN','hardware_proof':False,'timeout':False}
with (p/'qemu.log').open('w') as log:
 proc=subprocess.Popen(argv,stdout=log,stderr=subprocess.STDOUT)
 try: proc.wait(timeout=120)
 except subprocess.TimeoutExpired: r['timeout']=True;proc.terminate();proc.wait(timeout=5)
r['exit_code']=proc.returncode
text=(p/'serial.log').read_text();r['functional_migration']=assess_migration(text,2)
r['firmware']=[l for l in text.splitlines() if l.startswith('[T3R-FW]')]
r['cpu_identity']=[l for l in text.splitlines() if l.startswith('[CPU-ID]')]
r['serial_sha256']=hashlib.sha256(text.encode()).hexdigest();r['seconds']=time.monotonic()-start
if not any('[CPU-ID] logical=0 apic=0 bsp=true'==l for l in r['cpu_identity']): raise RuntimeError('Unexpected BSP: investigate, do not assume blocked')
r['reason']='Actual BSP remained APIC 0; firmware slot 0 is BSP. CPU property did not produce the requested hardware.'
(p/'result.json').write_text(json.dumps(r,indent=2)+'\n');print(p);print(json.dumps(r,indent=2))
