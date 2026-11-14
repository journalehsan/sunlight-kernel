from pathlib import Path
import subprocess,json,hashlib
E=Path(__file__).resolve().parent
elf=Path.cwd()/'target/r2b3-detached-accounting.elf'
base=['python3','tools/native-c/cc.py','-O2','-std=c11','-Wall','-Wextra','-Werror',str(E/'detached-accounting.c'),'-o',str(elf)]
rows=[]
address=0
for attempt in range(3):
 args=base+[f'-Wl,--defsym=__r3_queue={address}']
 p=subprocess.run(args,capture_output=True);(E/f'accounting-link-{attempt}.log').write_bytes(p.stdout+p.stderr)
 if p.returncode: raise SystemExit(p.returncode)
 syms=subprocess.check_output(['nm',str(elf)]).decode(); (E/f'accounting-symbols-{attempt}.log').write_text(syms)
 targets=[l for l in syms.splitlines() if 'sunlight_libc7pthread8DETACHED' in l]
 assert len(targets)==1,targets
 actual=int(targets[0].split()[0],16)
 rows.append(dict(command=args,exit=p.returncode,bound_address=address,actual_address=actual))
 if address==actual: break
 address=actual
else: raise SystemExit('unstable address')
(E/'accounting-build.json').write_text(json.dumps({'attempts':rows,'binding_verified':address==actual,'elf_sha256':hashlib.sha256(elf.read_bytes()).hexdigest(),'private_layout_source':'sunlight-libc/src/pthread.rs Control repr(C), size 96','production_source_changed':True},indent=2)+'\n')
