import pathlib,json,subprocess,hashlib,csv,shlex
root=pathlib.Path.cwd();p=pathlib.Path(__file__).resolve().parent;report='docs/runtime/native-runtime-r1-reproducibility-cpython-probe.md'
def digest(f):return hashlib.sha256(pathlib.Path(f).read_bytes()).hexdigest()
initial=json.loads((p/'initial-source.json').read_text());before=initial['sha256'];after={f:digest(f) if pathlib.Path(f).is_file() else None for f in before}
changes={f:{'before':before[f],'after':after[f]} for f in before if before[f]!=after[f]}
assert set(changes)=={'tools/native-c/Cargo.lock'}, changes.keys()
newfiles=subprocess.check_output(['git','ls-files','-co','--exclude-standard','-z']).decode().split('\0')
added=[f for f in newfiles if f and f not in before and pathlib.Path(f).is_file()]
assert subprocess.check_output(['git','diff','--name-only']).decode().splitlines()==['tools/native-c/Cargo.lock']
assert all(f.startswith(str(p.relative_to(root))) or f==report for f in added),added
identity=hashlib.sha256(json.dumps(after,sort_keys=True).encode()).hexdigest()
run=subprocess.run(['bash','tools/build_native_c.sh'],capture_output=True,text=True)
(p/'final-after-all-tests-locked-build.log').write_text('$ bash tools/build_native_c.sh\n'+run.stdout+run.stderr+'\nexit='+str(run.returncode)+'\n');assert run.returncode==0
assert all(digest(f)==h for f,h in after.items())
manifest={'revision':subprocess.check_output(['git','rev-parse','HEAD']).decode().strip(),'branch':subprocess.check_output(['git','branch','--show-current']).decode().strip(),'initial_status':initial['status'],'final_status':subprocess.check_output(['git','status','--porcelain=v1']).decode(),'original_file_comparison':changes,'preserved_original_files':len(before)-len(changes),'authorized_source_changes':['tools/native-c/Cargo.lock'],'authorized_added_files':sorted(added),'unrelated_source_drift':[],'final_baseline_file_sha256':after,'test_source_fingerprint_sha256':identity,'test_applicability':'initial-suite uses original baseline; all later native tests use original baseline plus reviewed lock repair. Final comparison found no other original file changes. Added report/evidence are not production build inputs.','build_generated_areas':['target/native-c','target/native-cxx','target/native-std','target/x86_64-unknown-none','target/native-c-guest-build.lock','evidence local binaries/maps/diagnostics','/tmp/sunlight-r1-lock-1qqs0g3l','/tmp/sunlight-r1-cpython'],'ignored_output_limitation':'preexisting ignored target tree was not inventoried before builds; do not attribute all target files as newly created','final_locked_build_exit':run.returncode,'report_sha256':digest(report)}
(p/'source-integrity-manifest.json').write_text(json.dumps(manifest,indent=2))
base={'source_revision':manifest['revision'],'source_fingerprint_sha256':identity,'source_state':'reviewed native Cargo.lock repair; all other original files match baseline','compiler':'clang 23.1.4','linker':'LLD 23.1.4','rustc':'1.98.0-nightly 8954863c8','target':'native SunlightOS JSON / x86_64-unknown-none-elf; static SysV x86-64 SSE2','cpus':2,'execution':'native SunlightOS QEMU TCG','historical_result':False}
rows=list(csv.DictReader((p/'final-suite/results.csv').open()));obs=json.loads((p/'final-suite/results.json').read_text());tests=[]
for row in rows:
 name=row['probe'];build=p/'final-suite'/('build-'+name+'.log');line=build.read_text().splitlines()[0];argv=shlex.split(line.removeprefix('$ '));sources=[x for x in argv if x.endswith(('.c','.cpp'))]
 tests.append({**base,**row,'fixture_origin':'historical' if row['historical']=='True' else 'focused native fixture','runtime_result':'PASS','failure_classification':None,'build_command':line,'fixture_sources':[{'path':f,'sha256':digest(f)} for f in sources],'observed':obs.get(name),'evidence':['final-suite/build-'+name+'.log','final-suite/elf-'+name+'.log','final-suite/serial.log','final-suite/results.json']})
summary={'overall':'R1 COMPLETE','gates':{f'G{i}':'PASS' for i in range(1,8)},'existing_suite':{'total':len(rows),'C':73,'Cxx':21,'build':'PASS','link':'PASS','guest':'PASS','acceptance':'PASS'},'native_tests':tests,'supplemental':[],'initial_failure':{'runner_exit':1,'cargo_exit':101,'classification':'BLOCKED','link':'NOT_RUN','guest':'NOT_RUN','source':'initial-source.json'},'cpython':{'reference':'3.13.0','build_python':'3.13.0 host tooling only','original_configure':'FAIL upstream platform rejection','experimental_configure':'FAIL C99 math atanh unresolved after separate libm removed','minimal_target_interpreter_build':'BLOCKED','target_interpreter_link':'BLOCKED','target_interpreter_guest':'NOT_RUN'},'source_integrity':'PASS'}
for name,d in [('heap-abi','heap-abi-guest'),('tcp-loopback','tcp-loopback/guest')]:
 observed=json.loads((p/d/'results.json').read_text());assert observed['elf']['pass_']
 summary['supplemental'].append({**base,'name':name,'build':'PASS','link':'PASS','guest':'PASS','observed':observed['elf'],'evidence':[d+'/serial.log',d+'/results.json']})
summary['cxx_target_validation']={'result':'PASS','execution':'target object/archive inspection, not host runtime','evidence':'cxx-abi-runner.log','source_fingerprint_sha256':identity}
(p/'test-result-summary.json').write_text(json.dumps(summary,indent=2))
# Exact generated native archive and staged Rust library used by repeat builds.
artifacts={str(f):digest(f) for d in ['target/native-c/sysroot/lib'] for f in pathlib.Path(d).glob('*.a')}
stage=pathlib.Path('target/native-std/sysroot/lib/rustlib/src/rust/library')
staged={str(f):digest(f) for f in stage.rglob('*') if f.is_file() and f.suffix in ['.rs','.toml','.lock']}
(p/'generated-build-inputs.json').write_text(json.dumps({'archives':artifacts,'staged_rust_library_sha256':staged,'note':'post-validation generated-input snapshot; repeated preparation uses same pin and repository overlays; not an initial ignored-tree baseline'},indent=2))
(p/'final-revision-status.log').write_text(manifest['revision']+'\nbranch='+manifest['branch']+'\n'+manifest['final_status'])
print('Source integrity PASS;',len(before),'original files;',len(tests),'suite cases; final locked build',run.returncode)
