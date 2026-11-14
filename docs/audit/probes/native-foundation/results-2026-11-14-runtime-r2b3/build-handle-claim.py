from pathlib import Path
import subprocess,json,hashlib
E=Path(__file__).resolve().parent; elf=Path.cwd()/'target/r2b3-handle-claim.elf'
aliases={'__r3_controls':'sunlight_libc7pthread8CONTROLS','__r3_lock':'sunlight_libc7pthread13DETACHED_LOCK'}
addresses={a:0 for a in aliases};rows=[]
for attempt in range(3):
 args=['python3','tools/native-c/cc.py','-O2','-std=c11','-Wall','-Wextra','-Werror',str(E/'handle-claim.c'),'-o',str(elf)]+[f'-Wl,--defsym={a}={v}' for a,v in addresses.items()]
 p=subprocess.run(args,capture_output=True);(E/f'claim-link-{attempt}.log').write_bytes(p.stdout+p.stderr)
 if p.returncode: raise SystemExit(p.returncode)
 symbols=subprocess.check_output(['nm','-S',str(elf)]).decode();(E/f'claim-symbols-{attempt}.log').write_text(symbols)
 actual={}
 for a,pattern in aliases.items():
  matches=[line for line in symbols.splitlines() if pattern in line];assert len(matches)==1,matches
  fields=matches[0].split();actual[a]=int(fields[0],16)
  assert int(fields[1],16)==(1 if a=='__r3_lock' else 8),matches
 rows.append({'command':args,'exit':p.returncode,'bound':addresses,'actual':actual})
 if actual==addresses: break
 addresses=actual
else: raise SystemExit('unstable symbol binding')
(E/'claim-build.json').write_text(json.dumps({'attempts':rows,'binding_verified':actual==addresses,'elf_sha256':hashlib.sha256(elf.read_bytes()).hexdigest(),'runtime_layout':'Control repr(C)96 bytes; DETACHED_LOCK AtomicBool1 byte; CONTROLS usize8 bytes','runtime_code_patched':False},indent=2)+'\n')
