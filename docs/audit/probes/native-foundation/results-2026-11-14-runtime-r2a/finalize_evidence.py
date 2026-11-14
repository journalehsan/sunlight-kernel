import pathlib,subprocess,json,hashlib,os,csv,re
r=pathlib.Path.cwd();e=pathlib.Path(__file__).resolve().parent;report='docs/runtime/native-runtime-r2a-headers-libm-cpython-compile.md'
def digest(p):return hashlib.sha256(p.read_bytes()).hexdigest()
initial=json.loads((e/'initial-source.json').read_text());changes={};missing=[]
for name,old in initial['hashes'].items():
 p=r/name
 if not p.is_file():missing.append(name)
 elif (new:=digest(p))!=old:changes[name]=dict(before=old,after=new)
allowed={'Cargo.lock','sunlight-libc/Cargo.toml','sunlight-libc/src/lib.rs','sunlight-libc/include/errno.h','sunlight-libc/include/limits.h','sunlight-libc/include/math.h','sunlight-libc/include/sys/mman.h','sunlight-libc/include/time.h','sunlight-libc/include/unistd.h','tools/native-c/Cargo.lock'}
aliases={}
for name in changes.keys()-allowed:
 p=r/name
 assert p.is_symlink() and p.resolve()==r/'Cargo.lock',name
 git_target=subprocess.check_output(['git','show','HEAD:'+name],cwd=r).decode()
 assert os.readlink(p)==git_target,name
 aliases[name]=dict(kind='unchanged symlink; followed-content hash tracks authorized root lock change',target=os.readlink(p),git_blob_target=git_target)
assert not missing,missing
files=[s for s in subprocess.check_output(['git','ls-files','--cached','--others','--exclude-standard','-z'],cwd=r).decode().split('\0') if s and (r/s).is_file()]
added=[s for s in files if s not in initial['hashes']]
production_added=[s for s in added if not s.startswith(str(e.relative_to(r))+'/') and s!=report]
assert all(s.startswith('sunlight-libc/vendor/libm/') or s in {'sunlight-libc/src/math.rs','sunlight-libc/src/inttypes.rs','sunlight-libc/src/ctype.rs','sunlight-libc/include/assert.h','sunlight-libc/include/inttypes.h','sunlight-libc/include/sys/types.h','sunlight-libc/include/ctype.h'} for s in production_added),production_added
vendor_original=json.loads((e/'libm-vendor-integrity.json').read_text());vendor_changed=[s for s,h in vendor_original.items() if digest(r/'sunlight-libc/vendor/libm'/s)!=h];assert vendor_changed==['Cargo.toml'],vendor_changed
# Fingerprint actual runtime/compiler/kernel inputs, excluding documentation and output.
prefixes=('sunlight-libc/','sunlight-cxx/','ipc/','kernel/','tools/native-c/','tools/native-std/','tools/targets/')
inputs={s:digest(r/s) for s in files if s.startswith(prefixes) or s in {'Cargo.lock','Cargo.toml','rust-toolchain.toml','.cargo/config.toml','tools/build_native_c.sh','tools/build_native_cxx.sh','tools/build_native_std.sh'}}
fingerprint=hashlib.sha256(json.dumps(inputs,sort_keys=True).encode()).hexdigest();(e/'final-runtime-inputs.json').write_text(json.dumps(dict(fingerprint_sha256=fingerprint,files=inputs),indent=2))
manifest=dict(revision=initial['revision'],branch=initial['branch'],initial_status=initial['status'],final_status=subprocess.check_output(['git','status','--short'],cwd=r).decode(),original_file_changes=changes,missing_original_files=missing,unexpected_original_changes=[],symlink_aliases=aliases,preserved_original_files=len(initial['hashes'])-len(changes),authorized_added_production_files=production_added,report=report,report_sha256=digest(r/report),all_R1_files_preserved=True,kernel_scheduler_pthread_production_unchanged=True,vendor_original_modified_files=vendor_changed,numerical_vendor_sources_unchanged=True,runtime_fingerprint_sha256=fingerprint)
(e/'source-integrity-manifest.json').write_text(json.dumps(manifest,indent=2))
(e/'source-diff.patch').write_bytes(subprocess.check_output(['git','diff','--','Cargo.lock','sunlight-libc','tools/native-c/Cargo.lock'],cwd=r))
rows=list(csv.DictReader((e/'acceptance-suite/results.csv').open()));assert len(rows)==94
assert all(x['build']=='PASS' and x['guest_run']=='PASS' and x['acceptance']=='PASS' for x in rows)
r1=json.loads((e.parent/'results-2026-11-14-runtime-r1/test-result-summary.json').read_text())
cpp={x['probe'] for x in r1['native_tests'] if any(s['path'].endswith(('.cpp','.cc','.cxx')) for s in x.get('fixture_sources',[]))}
assert len(cpp)==21,(len(cpp),cpp)
assert len([x for x in rows if x['probe'] in cpp])==21
for row in rows:
 row.update(language='C++' if row['probe'] in cpp else 'C',source_revision=initial['revision'],runtime_fingerprint_sha256=fingerprint,cpus=2,execution='SunlightOS QEMU TCG',historical_execution=False)
focused=json.loads((e/'acceptance-focused/focused-results.json').read_text());assert len(focused)==13 and all(x['status']=='PASS' for x in focused)
for label in ['final-math','final-headers-c','final-headers-cxx','final-ctype','final-headers-c-assert','final-headers-cxx-assert']:
 d=json.loads((e/'acceptance-focused'/(label+'-guest')/'results.json').read_text());assert d['elf']['pass_'],label
heap=json.loads((e/'acceptance-heap-abi-guest/results.json').read_text());assert heap['elf']['pass_'] and 'R1 heap and SysV ABI PASS' in heap['elf']['stdout']
tcp=json.loads((e/'acceptance-tcp/poll-summary.json').read_text());assert tcp['result']=='PASS' and all(tcp['checks'].values())
assert 'ownership, upstream integrity, final ELF/frame inspection PASS' in (e/'acceptance-cxx-abi-runner.log').read_text()
for f in ['baseline-locked-build.log','acceptance-locked-build.log','final-locked-repeat.log']:assert 'Finished `release`' in (e/f).read_text(),f
config=json.loads((e/'cross-acceptance-result.json').read_text());assert config['exit']==0
compile=json.loads((e/'acceptance-compile-results.json').read_text());assert len(compile)==6 and all(x['exit']==2 for x in compile)
assert 'pthread_key_t' in (e/'acceptance-compile-Programs-python-o.log').read_text()
assert json.loads((e/'cpython-compatibility-inventory.json').read_text())['actual_compile_findings']
summary=dict(overall='R2A COMPLETE; CPython NOT BUILT',gates={f'G{i}':'PASS' for i in range(1,8)},source_revision=initial['revision'],branch=initial['branch'],runtime_fingerprint_sha256=fingerprint,locked_build='PASS (baseline, final, repeated)',existing_suite=dict(C=73,Cxx=21,total=94,cpus=2,build='PASS',link='PASS',guest='PASS',acceptance='PASS',evidence='acceptance-suite/results.csv'),legacy_cases=rows,cxx_abi=dict(status='PASS',evidence='acceptance-cxx-abi-runner.log'),heap_abi=dict(status='PASS',evidence='acceptance-heap-abi-guest/results.json'),tcp_loopback=dict(status='PASS',evidence='acceptance-tcp/poll-summary.json'),new_tests=dict(focused_guest_scenarios=6,focused_checks_including_compiles_builds=13,math_vectors=17342,ctype_input_function_pairs=3598,format_macro_type_combinations_per_language=154,status='PASS',evidence='acceptance-focused/focused-results.json'),cpython=dict(version='3.13.0',configure='PASS',compile='FAIL (expected out-of-scope blockers; G6 PASS for actual diagnostic attempt)',object_targets=5,interpreter_make_attempt=True,first_translation_unit='Programs/python.c',first_error='unknown type pthread_key_t at Include/cpython/pythread.h:37',static_interpreter_link='NOT_RUN (compile blocked)',execution='NOT_RUN',evidence=['cross-acceptance-config.log','acceptance-compile-results.json']),integrity=dict(status='PASS',evidence='source-integrity-manifest.json'),report=report)
(e/'validation-summary.json').write_text(json.dumps(summary,indent=2))
# Omit large generated images/binaries, but hash every retained textual evidence file.
checks={str(p.relative_to(e)):digest(p) for p in e.rglob('*') if p.is_file() and p.suffix in {'.json','.log','.txt','.csv','.py','.c','.h','.patch','.md'} and p.name!='evidence-checksums.json'}
(e/'evidence-checksums.json').write_text(json.dumps(checks,indent=2));print(json.dumps(dict(overall=summary['overall'],gates=summary['gates'],source_changes=list(changes),new_production_files=len(production_added),legacy_C=73,legacy_Cxx=21,focused_guest_cases=6,math_vectors=17342),indent=2))
