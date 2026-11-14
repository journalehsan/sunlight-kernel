"""Reuse unchanged native KFP assessor in a fresh evidence directory.
These are existing retirement controls, never a native thread-group W9 claim.
"""
import pathlib,json,hashlib,importlib.util,os,shutil,subprocess
ROOT=pathlib.Path(__file__).resolve().parents[5]
OUT=pathlib.Path(os.environ.get('R2B5_OUTPUT',str(pathlib.Path(__file__).resolve().parent)))/'retirement-controls';OUT.mkdir(exist_ok=False)
(OUT/'.gitignore').write_text('*\n!*.log\n!*.json\n!*.py\n!.gitignore\n')
spec=importlib.util.spec_from_file_location('kfp3_controls',ROOT/'.kfp-2a3-evidence/run_guests.py');pilot=importlib.util.module_from_spec(spec);spec.loader.exec_module(pilot);pilot.OUT=OUT
paths=[*list((ROOT/'kernel').rglob('*.rs')),*list((ROOT/'ipc').rglob('*.rs')),*[ROOT/p for p in ['Cargo.lock','Cargo.toml','kernel/Cargo.toml','kernel/build.rs','ipc/Cargo.toml']]]
source={'head':subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),'sha256':{str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest() for p in paths}}
pilot.SOURCE=source;(OUT/'source-build.json').write_text(json.dumps(source,indent=2)+'\n');rows=[]
for mode in [0,2,3]:
 pilot.integrity();env={**os.environ,'KFP2A2_FAULT':str(mode),'CARGO_TARGET_DIR':str(ROOT/'target/r2b5-retirement')}
 feature='mm2d_munmap_test' if mode==0 else 'kfp2a2_fault_test'
 pilot.run('fi'+str(mode)+'-build',['cargo','build','--locked','--offline','-p','sunlight-kernel','--bin','sunlight-kernel','--features',feature],env)
 elf=OUT/('fi'+str(mode)+'.elf');iso=OUT/('fi'+str(mode)+'.iso');scratch=OUT/('fi'+str(mode)+'-iso-work');scratch.mkdir();shutil.copy2(ROOT/'limine.conf',scratch/'limine.conf')
 pilot.run('fi'+str(mode)+'-freeze',['objcopy','--strip-debug',str(ROOT/'target/r2b5-retirement/x86_64-unknown-none/debug/sunlight-kernel'),str(elf)])
 pilot.run('fi'+str(mode)+'-iso',['bash','tools/make_hybrid_iso.sh',str(elf),str(iso),str(ROOT/'target/limine'),str(scratch)])
 pilot.integrity();row=pilot.guest(mode,iso,elf);rows.append(row);(OUT/'guest-results.json').write_text(json.dumps({'scope':'existing 2-CPU retirement controls; NOT a native thread-group W9 witness','guests':rows},indent=2)+'\n');print(mode,row['status'],row['checks'],flush=True)
 if row['status']!='PASS':break
pilot.integrity()
