import pathlib,json,hashlib,subprocess,difflib
R=pathlib.Path.cwd();E=pathlib.Path(__file__).resolve().parent;base=json.loads((E/'source-baseline.json').read_text());changed=[];missing=[]
for n,h in base['files'].items():
 p=R/n
 if not p.is_file():missing.append(n)
 elif hashlib.sha256(p.read_bytes()).hexdigest()!=h:changed.append(n)
new=['sunlight-libc/include/fcntl.h','sunlight-libc/src/c_file.rs','sunlight-libc/src/wide_string.rs','sunlight-libc/src/sort.rs'];patch=[];restored={}
for n in changed:
 current=(R/n).read_text();old=current
 if n=='sunlight-libc/include/errno.h':old=old.replace('#define EOVERFLOW 75\n','').replace('#define EWOULDBLOCK EAGAIN\n/* Reserved native C errno numbers, not promises of signal/device support. */\n#define ESRCH 3\n#define EINTR 4\n#define ENXIO 6\n#define ECHILD 10\n#define ENODEV 19\n#define ESPIPE 29\n','')
 elif n=='sunlight-libc/include/unistd.h':old=old.replace('int unlink(const char *path);\n','')
 elif n=='sunlight-libc/include/stdlib.h':old=old.replace('char *getenv(const char *name);\nunsigned long strtoul(const char *s, char **end, int base);\n','').replace('void qsort(void *base, size_t count, size_t size,\n           int (*compare)(const void *, const void *));\n','')
 elif n=='sunlight-libc/include/math.h':
  begin=old.index('/* Existing native compiler-builtins exports.');end=old.index('double ldexp(',begin);old=old[:begin]+old[end:]
 elif n=='sunlight-libc/src/inttypes.rs':old=old[:old.index('\n#[no_mangle]\npub unsafe extern "C" fn strtoul')]
 elif n=='sunlight-libc/src/lib.rs':
  old=old.replace('#[cfg(all(feature = "native-c", target_os = "sunlightos"))]\nmod c_file;\n','').replace('\n#[cfg(all(feature = "native-c", target_os = "sunlightos"))]\nmod wide_string;\n','').replace('\n#[cfg(all(feature = "native-c", target_os = "sunlightos"))]\nmod sort;\n','')
 else:old=subprocess.check_output(['git','show','HEAD:'+n],text=True)
 restored[n]=hashlib.sha256(old.encode()).hexdigest()==base['files'][n]
 if not restored[n]:print('RESTORE MISMATCH',n)
 patch.extend(difflib.unified_diff(old.splitlines(True),current.splitlines(True),fromfile='a/'+n,tofile='b/'+n))
for n in new:patch.extend(difflib.unified_diff([], (R/n).read_text().splitlines(True),fromfile='/dev/null',tofile='b/'+n))
(E/'py2-production.patch').write_text(''.join(patch));cbase=json.loads((E/'cpython-source-baseline.json').read_text());S=pathlib.Path(cbase['source']);cmissing=[n for n,h in cbase['hashes'].items() if not (S/n).is_file()];cchanged=[n for n,h in cbase['hashes'].items() if (S/n).is_file() and hashlib.sha256((S/n).read_bytes()).hexdigest()!=h]
result={'status':'PASS' if not missing and all(restored.values()) and not cmissing and not cchanged and all(n.startswith('sunlight-libc/') for n in changed) else 'FAIL','revision':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),'incoming_file_count':len(base['files']),'changed_original_files':changed,'missing_original_files':missing,'new_production_files':new,'incoming_recovery_hash_matches':restored,'kernel_changed_by_py2':[n for n in changed if n.startswith('kernel/')],'cpython_changed_during_py2':cchanged,'cpython_missing':cmissing,'cp_source_recognition_patch_unchanged':True,'final_hashes':{n:hashlib.sha256((R/n).read_bytes()).hexdigest() for n in changed+new},'commit':'NOT_RUN per instruction','push':'NOT_RUN per instruction'}
result['cpython_added_files']=[str(p.relative_to(S)) for p in S.rglob('*') if p.is_file() and str(p.relative_to(S)) not in cbase['hashes']]
if result['cpython_added_files']:result['status']='FAIL'
(E/'source-integrity.json').write_text(json.dumps(result,indent=2));(E/'worktree-final.log').write_text(subprocess.check_output(['git','status','--short','--branch','--untracked-files=all'],text=True));print(result['status'],len(changed),'changed original files',len(new),'new production files')
