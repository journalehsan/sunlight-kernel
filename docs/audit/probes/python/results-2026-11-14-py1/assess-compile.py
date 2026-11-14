import pathlib,re,json,hashlib,shlex
E=pathlib.Path(__file__).resolve().parent; B=pathlib.Path('/tmp/sunlight-py1-cpython/cross-static'); S=B.parent/'Python-3.13.0'
log=(E/'core-compile-verified.log').read_text();units={}
for l in log.replace('\\\n',' ').splitlines():
 if 'python3 /home/ehsantor/Projects/sunlightos-kernel/tools/native-c/cc.py ' in l and ' -c ' in l and '-o ' in l:
  l=l[l.index('python3 '):]
  args=shlex.split(l)
  o=args[args.index('-o')+1]
  sources=[x for x in args if x.endswith('.c')]
  if sources:units[o]={'object':o,'source':sources[-1],'command':l}

failed=set(re.findall(r'make: \*\*\* \[.*?: ([^\]]+\.o)\] Error',log))
for o,u in units.items():
 u['status']='FAIL' if o in failed else 'PASS' if (B/o).is_file() else 'UNKNOWN'
 p=pathlib.Path(u['source']);p=p if p.is_absolute() else B/p
 if not p.exists():p=S/u['source']
 u['source_sha256']=hashlib.sha256(p.read_bytes()).hexdigest() if p.exists() else None
text=(E/'configure-verified-Makefile').read_text().replace('\\\n',' ');v={}
for l in text.splitlines():
 m=re.match(r'^([A-Za-z0-9_]+)\s*=\s*(.*)',l)
 if m:v[m[1]]=m[2]
def expand(s,depth=0):
 if depth>30:return s
 s=re.sub(r'\$\{(\w+)\}',lambda m:expand(v.get(m[1],''),depth+1),s)
 return re.sub(r'\$\((\w+)\)|\$(\w)',lambda m:expand(v.get(m[1] or m[2],''),depth+1),s)
expected=sorted(set(expand(v['LIBRARY_OBJS']).split()+['Programs/python.o']));not_attempted=sorted(set(expected)-set(units));errors=[l for l in log.splitlines() if 'error:' in l]
serial=(E/'core-compile-verified-serial.log').read_text()
summary={'command':json.loads((E/'core-compile-verified-result.json').read_text())['command'],'exit':2,'configured_object_targets':len(expected),'attempted_translation_units':len(units),'successful_translation_units':sum(u['status']=='PASS' for u in units.values()),'failed_translation_units':sum(u['status']=='FAIL' for u in units.values()),'not_attempted_targets':not_attempted,'first_parallel_error':errors[0] if errors else None,'first_serial_error':next((l for l in serial.splitlines() if 'error:' in l),None),'errors':errors,'diagnostic_limit':'Default Clang error limit: some failing units stop after 20 diagnostics. Inventory is not exhaustive.','units':list(units.values())}
(E/'compile-summary.json').write_text(json.dumps(summary,indent=2));(E/'core-static-object-manifest.txt').write_text('\n'.join(expected)+'\n')
inventory={'missing_headers':sorted(set(re.findall(r"fatal error: '([^']+)' file not found",log))),'missing_declarations':sorted(set(re.findall(r"call to undeclared function '([^']+)'",log))),'missing_identifiers':sorted(set(re.findall(r"use of undeclared identifier '([^']+)'",log))),'incomplete_types':sorted(set(re.findall(r"(?:incomplete definition of type|variable has incomplete type) '([^']+)'",log))),'abi_mismatches':'No demonstrated wire ABI mismatch; conflicting prototypes follow incomplete public types. New condvar C/C++ layout assertions pass.','unsupported_compiler_features':'None observed in captured diagnostics','link_unresolved_symbols':'NOT_RUN: no interpreter link reached; missing declarations are not a linker inventory','first_blocker':summary['first_serial_error']}
(E/'libc-gap-inventory.json').write_text(json.dumps(inventory,indent=2))
base=json.loads((E/'cpython-pristine.json').read_text());changed=[];missing=[]
for n,h in base.items():
 p=S/n
 if not p.is_file():missing.append(n)
 elif hashlib.sha256(p.read_bytes()).hexdigest()!=h:changed.append(n)
added=sorted(str(p.relative_to(S)) for p in S.rglob('*') if p.is_file() and str(p.relative_to(S)) not in base)
(E/'cpython-source-integrity.json').write_text(json.dumps({'changed':changed,'missing':missing,'added_files':added,'expected_changes':['config.sub','configure'],'matches':set(changed)=={'config.sub','configure'} and not missing and not added,'patched_file_hashes':{n:hashlib.sha256((S/n).read_bytes()).hexdigest() for n in changed}},indent=2))
(E/'link-status.json').write_text(json.dumps({'status':'NOT_RUN','reason':'Core compilation exits 2; mandatory objects absent','interpreter':None,'link_command':None,'archive_order':None,'unresolved_symbols':None,'duplicate_symbols':None,'relocation_failures':None,'crt':'native archive _start -> sl_std_start -> C main; exercised by native fixtures only','tls':'native toolchain static x86-64 TLS; CPython link not reached'},indent=2))
(E/'interpreter-execution.json').write_text(json.dumps({'loader_startup':'NOT_RUN','first_python_statement':'NOT_RUN','first_python_output':None,'imports_and_runtime':'NOT_RUN','threaded_python':'NOT_READY','production':'NOT_READY','reason':'No linked interpreter'},indent=2))
print({k:summary[k] for k in ['configured_object_targets','attempted_translation_units','successful_translation_units','failed_translation_units']});print(inventory['missing_headers']);print(summary['first_serial_error'])
